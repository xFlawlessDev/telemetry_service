# Telemetry Service

Windows activation background agent written in Rust.

## Behavior

- Starts automatically at user logon via a self-deleting Scheduled Task.
- Loads existing local activation state, or keeps new state in memory until server success.
- Collects hardware serial number.
- Collects optional Windows geolocation coordinates.
- Optionally suppresses all POSTs while the device is inside a configured coordinate block zone.
- Posts activation payload to `https://register.axiooworld.com/axioo_on/create`.
- Retries retryable network/server failures with exponential backoff and jitter.
- Marks local state as activated after server success.
- Removes its startup task after successful activation.
- When launched by the scheduled task (`--self-delete-on-success`), also deletes its installed executable and folder.
- Manual runs (`--once`, `--dry-run`, or without the flag) never delete the binary.


## Activation Logic

Startup flow:

1. Discover data, state, and log paths.
2. Initialize file logging.
3. Load `activation_state.json` if it already exists; otherwise keep fresh state in memory only.
4. If `activated = true`, remove the startup task and exit.
5. Increment in-memory `attempt_count` and set `last_attempt_utc`.
6. Collect hardware serial number.
7. Collect optional Windows geolocation with timeout.
8. If a block zone is configured and the device is inside it, sleep and re-check without posting.
9. Build activation payload.
10. `POST` payload to `https://register.axiooworld.com/axioo_on/create`.
11. On API success (`result = 0`), store local state, remove the startup task, and exit.
12. On retryable failure, keep local state unsaved, sleep with backoff, then retry.
13. On fatal failure, keep local state unsaved and exit with error.

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
- API business reject (`result = -1`), e.g. invalid/expired token, empty serial number, database save failure
- unexpected non-retryable client errors

Backoff starts at 15 seconds, caps at 15 minutes, and applies ±20% jitter. Default mode retries forever because startup activation must survive offline boot. `--once` changes retryable failure behavior to exit after one attempt without writing local state.

## Activation Request

Endpoint:

```text
POST https://register.axiooworld.com/axioo_on/create
```

Headers:

```text
Authorization: Bearer {token}
Content-Type: multipart/form-data
Idempotency-Key: {install_id}
```

`Idempotency-Key` uses the in-memory or persisted `install_id`, so duplicate create attempts are safe when the server implements idempotency. If no state file exists yet, the id becomes durable only after successful activation.

Payload fields:

```text
serial_number={hardware serial from Win32_BIOS/Win32_BaseBoard/Win32_ComputerSystemProduct}
latitude={Windows geolocation latitude}
longitude={Windows geolocation longitude}
accuracy_meters={Windows geolocation accuracy}
```

Only `serial_number`, `latitude`, `longitude`, and `accuracy_meters` are posted to the create endpoint. All payload fields are sent as form-data text fields.

Coordinate fields are nullable as text. If the hardware or Windows geolocation API does not support coordinates, the request is still sent with `latitude`, `longitude`, and `accuracy_meters` set to text value `null`.

Success response:

```json
{
  "result": 0,
  "message": "Save succeed"
}
```

Only HTTP `200 OK` with JSON `result = 0` marks local state activated. Other `200 OK` responses are treated as failed activation and use `message` as the server reason.
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

The state file is created only after the create endpoint returns success. Blocked domains, offline manufacturing networks, retryable failures, and fatal server responses do not create or update local activation state.

Corrupt state files are renamed to:

```text
activation_state.json.corrupt.<timestamp>
```

A fresh state file is then created only after the next successful activation.

## Logs

Default log path:

```text
%ProgramData%\TelemetryService\logs\activation.log
```

Logs include startup, state status, HTTP status, retry delays, activation success, and autostart cleanup result. Logs avoid API keys and raw auth headers.

## Configuration

Build-time values come from `.env` or process environment and are embedded into the binary by `build.rs`.

Supported keys:

```text
TELEMETRY_BASE_URL=https://activation.example.com
TELEMETRY_USER_ID=replace-with-build-time-user-id
TELEMETRY_API_KEY=replace-with-real-key
TELEMETRY_TASK_NAME=TelemetryServiceActivation
```

Environment variables passed to `cargo build` override values from `.env`. If a key is missing, `src/config.rs` uses safe development placeholders.

### Coordinate Block Zone

The agent can suppress all POSTs while the device is physically inside a circular zone (for example, a manufacturing floor where devices must not activate yet).

```text
TELEMETRY_BLOCK_LATITUDE=-6.2
TELEMETRY_BLOCK_LONGITUDE=106.816666
TELEMETRY_BLOCK_RADIUS_METERS=1500
```

All three values are required to enable the zone; if any is missing or invalid, the block zone is disabled. The distance is computed with the Haversine formula against the current Windows geolocation fix. While the device is inside the radius, the agent logs the decision, sleeps for 5 minutes (`block_zone_poll_interval`), re-reads the location, and never sends the activation request. If coordinates are unavailable while the zone is configured, the POST is also suppressed to avoid activating a device that may be inside the zone.

Other runtime defaults live in `src/config.rs`:

- `agent_version`
- request timeout
- retry backoff and jitter

`TELEMETRY_API_KEY` is compiled into the executable. It is not a real secret once shipped. Server-side rate limiting, idempotency, replay protection, and validation are still required.

## CLI Flags

```text
--once
```

Run one activation attempt, then exit on retryable failure without writing local state.

```text
--print-payload
```

Print the serialized activation payload to stdout and the log file for debugging. Debug builds show the payload in the console; release builds are console-less, so read the payload from `%ProgramData%\TelemetryService\logs\activation.log`.

```text
--dry-run
```

Print the activation payload and exit without posting to the API and without writing activation state or logs. Release builds are console-less, so run `scripts\test.cmd` or redirect stdout to a file.

```text
--install-startup
```

Register a `TelemetryServiceActivation` Scheduled Task that runs at logon of any user, in that user's own session (`BUILTIN\Users` principal, interactive token). The task security descriptor is extended so the non-elevated agent can delete the task after activation, so runs are silent and require no UAC prompt. It also grants `BUILTIN\Users` delete-only access to the installed binary and folder, so the agent can remove the app itself. Must run elevated, because logon-trigger tasks need administrator rights. Install also removes any leftover registry `Run` entry from older versions.

```text
--remove-startup
```

Delete the startup Scheduled Task. Missing task is treated as success, and any leftover registry `Run` entry is removed too.

```text
--self-delete-on-success
```

After activation succeeds, delete the installed executable and its folder. This flag is embedded in the Scheduled Task action; manual runs without it keep the binary in place. The app is removed only on success, so retrying offline is unaffected.

```text
--reset-state
```

Delete local activation state and logs. Use this before sealing or cloning a Windows image.

## Manufacturing Deploy

Safe image rule: copy the binary into the image, but do not keep local state from the master image. Manufacturing networks should block `register.axiooworld.com` during production so activation cannot post before the device reaches the intended activation network.

For Audit/OOBE or post-clone setup:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

For QC cleanup after a manual test run:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

Do not allow successful activation on the master image. Otherwise every clone can inherit activated local state.


Auto deploy script:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe -SourceExe .\telemetry_service.exe
```

See `docs/deployment-guide.md` for User Mode master, post-clone, Audit/OOBE, and QC cleanup flows.
## Autostart (Scheduled Task)

The startup task is `TelemetryServiceActivation` under the Task Scheduler root folder. `--install-startup` registers it with:

- a logon trigger that fires for any user;
- a `BUILTIN\Users` (`S-1-5-32-545`) principal with the interactive-token logon type, so the task runs in the session of the user who logs on;
- an extended security descriptor that grants Authenticated Users read/execute plus `DELETE` on the task itself.

Because the task grants `DELETE`, the non-elevated agent can remove its own task after successful activation. This is what makes the flow silent: no UAC prompt, no leftover startup entry, and no reliance on a later admin logon.

Install also runs `icacls` to grant `BUILTIN\Users` delete-only rights (`D,DC`) on the install folder and binary. Delete-only means the user can remove the binary but cannot replace it, so there is no privilege escalation. Once the task action receives `--self-delete-on-success`, the agent also spawns a hidden helper that waits for the process to exit and then deletes the binary and folder.

Install requires elevation because logon-trigger tasks are privileged. The agent itself always runs non-elevated.

The task name also matches the legacy registry `Run` value; install and remove delete any such value from both `HKLM` and `HKCU`, so older deployments do not keep starting the agent.

Verify the task:

```powershell
schtasks /Query /TN "TelemetryServiceActivation" /V /FO LIST
```

Inspect the security descriptor that allows non-admin deletion:

```powershell
$svc = New-Object -ComObject Schedule.Service; $svc.Connect()
$svc.GetFolder('\').GetTask('TelemetryServiceActivation').GetSecurityDescriptor(0xF)
```

Inspect the delete-only file permissions:

```powershell
icacls "C:\Program Files\TelemetryService"
```

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
