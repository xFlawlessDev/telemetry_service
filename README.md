# Telemetry Service

Windows activation background agent written in Rust.

## Behavior

- Starts under a Windows Scheduled Task at boot or login.
- Loads or creates local activation state.
- Collects hardware identity with WMI.
- Collects optional Windows geolocation.
- Posts activation payload to `{base_url}/device-activations`.
- Retries retryable network/server failures with exponential backoff and jitter.
- Marks local state as activated after server success.
- Deletes the activation Scheduled Task after success.
- Does not self-delete its own executable.


## Activation Logic

Startup flow:

1. Discover data, state, and log paths.
2. Initialize file logging.
3. Load `activation_state.json`; create it when missing.
4. If `activated = true`, delete the Scheduled Task and exit.
5. Increment `attempt_count`, set `last_attempt_utc`, and save state.
6. Collect WMI hardware identity.
7. Collect optional Windows geolocation with timeout.
8. Build activation payload.
9. `POST` payload to `{base_url}/device-activations`.
10. On success, store `activation_id`, set `activated = true`, save state, delete Scheduled Task, and exit.
11. On retryable failure, store `last_error`, save state, sleep with backoff, then retry.
12. On fatal failure, store `last_error`, save state, and exit with error.

Retryable failures:

- request timeout
- DNS/connect/request failure
- invalid response JSON
- HTTP `429`, with `Retry-After` respected when present
- HTTP `5xx`

Fatal failures:

- HTTP `400`
- HTTP `401`
- HTTP `403`
- unexpected non-retryable client errors

Backoff starts at 15 seconds, caps at 15 minutes, and applies ±20% jitter. Default mode retries forever because startup activation must survive offline boot. `--once` changes retryable failure behavior to save state and exit after one attempt.

## Activation Request

Endpoint:

```text
POST {base_url}/device-activations
```

Headers:

```text
Authorization: Bearer {api_key}
Content-Type: application/json
Idempotency-Key: {install_id}
```

`Idempotency-Key` uses the stable local `install_id`, so duplicate attempts are safe when the server implements idempotency.

Payload shape:

```json
{
  "install_id": "68206efe-35f5-4907-9bd1-0bb12c640971",
  "agent_version": "0.1.0",
  "hardware": {
    "bios_serial": "ABC123",
    "system_uuid": "03000200-0400-0500-0006-000700080009",
    "baseboard_serial": "ABC123",
    "manufacturer": "MSI",
    "model": "ADLP"
  },
  "location": {
    "access_status": "GeolocationAccessStatus(1)",
    "latitude": -6.2328915,
    "longitude": 106.895924,
    "accuracy_meters": 323.0,
    "timestamp_utc": "134258745650932211",
    "error": null
  },
  "attempt": {
    "count": 1,
    "first_seen_utc": "2026-06-14T01:36:04.5319993Z",
    "last_attempt_utc": "2026-06-14T01:36:04.5390905Z"
  }
}
```

Hardware fields are optional. Invalid placeholder identifiers are normalized to `null`, including empty strings, `To be filled by O.E.M.`, `Default string`, `System Serial Number`, and all-zero UUIDs.

Location fields are optional. Permission denial, unavailable service, timeout, or Windows API failure does not block activation; coordinates become `null` and `error` records the failure reason.

Success response:

```json
{
  "status": "activated",
  "activation_id": "server-generated-id"
}
```

Only `status = "activated"` with a non-empty `activation_id` marks local state activated.
## Local State

Default state path:

```text
%ProgramData%\TelemetryService\activation_state.json
```

Fallback path when `%ProgramData%` is unavailable:

```text
%LOCALAPPDATA%\TelemetryService\activation_state.json
```

If both are unavailable, the agent uses `./TelemetryService/activation_state.json`.

Corrupt state files are renamed to:

```text
activation_state.json.corrupt.<timestamp>
```

A fresh state file is then created.

## Logs

Default log path:

```text
%ProgramData%\TelemetryService\logs\activation.log
```

Logs include startup, state status, hardware/geolocation summaries, HTTP status, retry delays, activation success, and autostart cleanup result. Logs avoid API keys and raw auth headers.

## Configuration

Build-time values come from `.env` or process environment and are embedded into the binary by `build.rs`.

Supported keys:

```text
TELEMETRY_BASE_URL=https://activation.example.com
TELEMETRY_API_KEY=replace-with-real-key
TELEMETRY_TASK_NAME=TelemetryServiceActivation
```

Environment variables passed to `cargo build` override values from `.env`. If a key is missing, `src/config.rs` uses safe development placeholders.

Other runtime defaults live in `src/config.rs`:

- `agent_version`
- request timeout
- geolocation timeout
- retry backoff and jitter

`TELEMETRY_API_KEY` is compiled into the executable. It is not a real secret once shipped. Server-side rate limiting, idempotency, replay protection, and validation are still required.

## CLI Flags

```text
--once
```

Run one activation attempt, save attempt metadata, then exit on retryable failure.

```text
--print-payload
```

Print the serialized activation payload to stdout for debugging.

```text
--install-task
```

Create the `ONLOGON` Scheduled Task for the current executable path.

```text
--remove-task
```

Delete the Scheduled Task. Missing task is treated as success.

```text
--reset-state
```

Delete local activation state and logs. Use this before sealing or cloning a Windows image.

## Manufacturing Deploy

Safe image rule: copy the binary into the image, but do not keep local state from the master image.

For Audit/OOBE or post-clone setup:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-task
```

For QC cleanup after a manual test run:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-task
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-task
```

Do not run activation on the master image unless state is reset afterward. Otherwise every clone can inherit the same `install_id`.


Auto deploy script:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe -SourceExe .\telemetry_service.exe
```

See `docs/deployment-guide.md` for User Mode master, post-clone, Audit/OOBE, and QC cleanup flows.
## Scheduled Task

Recommended install command:

```powershell
schtasks /Create /TN "TelemetryServiceActivation" /SC ONLOGON /RL LIMITED /TR "C:\Program Files\TelemetryService\telemetry_service.exe" /F
```

Alternative boot task:

```powershell
schtasks /Create /TN "TelemetryServiceActivation" /SC ONSTART /RL HIGHEST /TR "C:\Program Files\TelemetryService\telemetry_service.exe" /F
```

Cleanup performed by the agent after activation:

```powershell
schtasks /Delete /TN "TelemetryServiceActivation" /F
```

Missing scheduled task is treated as cleanup success.

## Development

Run checks:

```powershell
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Smoke run without waiting forever on retryable network failures:

```powershell
cargo run -- --once --print-payload
```

## Release Behavior

Debug builds keep a console window. Windows release builds use the Windows subsystem and do not show a console window.
