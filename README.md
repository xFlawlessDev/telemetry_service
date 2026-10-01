# Telemetry Service

Windows activation background agent written in Rust.

## Behavior

- Starts automatically at user logon via a self-deleting Scheduled Task.
- Writes no local files in production mode; no install id, state, timestamps, or logs.
- Collects hardware serial number.
- **Requires** Windows geolocation coordinates: force-enables location silently, and holds the POST until coordinates are available.
- Optionally suppresses all POSTs while the device is inside a configured coordinate block zone.
- Sends a structured JSON log of the activation run and the public IP (own endpoint, ipify fallback) alongside the coordinates when their feature flags are enabled.
- Posts activation payload to `https://register.axiooworld.com/axioo_on/create`.
- Retries retryable network/server failures with exponential backoff and jitter.
- Removes its startup task after successful activation; the task's `--self-delete-on-success` action then removes the binary and folder.
- Uses a deterministic install id derived from the hardware serial number so retries are idempotent without persisting state.
- Manual runs (`--once`, `--dry-run`, or without the flag) never delete the binary.
- Optional build-time debug mode (`TELEMETRY_DEBUG=1`) keeps JSON state and log files for troubleshooting.


## Activation Logic

Startup flow:

1. Discover the data directory, state, and log paths (used only in debug mode).
2. If debug JSON state exists with `activated = true`, remove the startup task and exit.
3. Collect hardware serial number.
4. **Force-enable Windows geolocation** (consent registry + `lfsvc` service) and wait until real coordinates are available, with timeout per attempt.
5. If a block zone is configured and the device is inside it, sleep and re-check without posting.
6. Build activation payload (serial, coordinates, plus `ip_public`/`logs` when their flags are enabled).
7. `POST` payload to `https://register.axiooworld.com/axioo_on/create`.
8. On API success (`result = 0`), remove the startup task and exit. In debug mode, write state first.
9. On retryable failure, sleep with backoff, then retry.
10. On fatal failure, exit with error.

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

Backoff starts at 15 seconds, caps at 15 minutes, and applies ±20% jitter. Default mode retries forever because startup activation must survive offline boot. `--once` changes retryable failure behavior to exit after one attempt without writing state.

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

`Idempotency-Key` is a deterministic install id derived from the hardware serial number (UUID v5), so duplicate create attempts are safe when the server implements idempotency. The same device always sends the same key, and nothing is persisted locally. In debug mode the key is a random `install_id` that is stored in the JSON state after successful activation.

Payload fields:

```text
serial_number={hardware serial from Win32_BIOS/Win32_BaseBoard/Win32_ComputerSystemProduct}
latitude={Windows geolocation latitude}
longitude={Windows geolocation longitude}
accuracy_meters={Windows geolocation accuracy}
ip_public={public IP, only when TELEMETRY_SEND_IP_PUBLIC is enabled}
logs={JSON array of structured log records, only when TELEMETRY_SEND_LOGS is enabled}
```

All payload fields are sent as form-data text fields. `serial_number`, `latitude`, `longitude`, and `accuracy_meters` are always present.

`ip_public` and `logs` are **opt-in** and omitted unless their build-time flags are enabled. When `TELEMETRY_SEND_IP_PUBLIC` is on, the agent tries the `TELEMETRY_IP_PUBLIC_URL` endpoint first, then falls back to `https://api.ipify.org` (both plain-text IP); if every lookup fails or times out it is sent as text `null`. When `TELEMETRY_SEND_LOGS` is on, `logs` is a JSON array string (empty `[]` when nothing was captured).

Location is a hard requirement. The agent force-enables Windows geolocation through the consent registry and the `lfsvc` service (needs an elevated token, see Autostart), and suppresses the POST while coordinates are unavailable. Only when a real fix is obtained is the request sent; the coordinate fields are therefore populated in normal operation.

Success response:

```json
{
  "result": 0,
  "message": "Save succeed"
}
```

Only HTTP `200 OK` with JSON `result = 0` counts as successful activation. Other `200 OK` responses are treated as failed activation and use `message` as the server reason.

## Local State

Production mode (default) writes **no local files at all**. Completion is not tracked on disk; instead the install id is derived deterministically from the hardware serial number (UUID v5), so retries reuse the same server-side `Idempotency-Key` and the agent stops re-activating only because the startup task and binary delete themselves after success. Blocked domains, offline manufacturing networks, retryable failures, and fatal server responses leave nothing behind.

### Debug State

With `TELEMETRY_DEBUG=1`, the agent keeps a durable JSON state file for troubleshooting:

```text
%ProgramData%\TelemetryService\activation_state.json
```

It records a random `install_id`, `activated`, `activation_id`, `attempt_count`, and timestamps, and is used to skip activation when `activated = true`. Corrupt JSON is renamed to `activation_state.json.corrupt.<timestamp>`.

## Logs

Production mode writes no log files. Tracing events are captured into an in-memory ring buffer (last `MAX_LOG_RECORDS`, currently 200) and shipped only when the `TELEMETRY_SEND_LOGS` feature flag is enabled, as the `logs` form field — a JSON array of `{timestamp_utc, level, target, message, fields}` records. Errors also go to stderr, which release builds discard because the binary runs without a console.

The buffer is snapshotted immediately before each POST, so retries include the events from previous attempts. It is bounded so a long retry loop cannot grow the request body without limit.

With `TELEMETRY_DEBUG=1`, the same events are additionally written to:

```text
%ProgramData%\TelemetryService\logs\activation.log
```

Debug logs include startup, state status, HTTP status, retry delays, activation success, and autostart cleanup result. Logs avoid API keys and raw auth headers.

## Configuration

Build-time values come from `.env` or process environment and are embedded into the binary by `build.rs`.

Supported keys:

```text
TELEMETRY_BASE_URL=https://activation.example.com
TELEMETRY_USER_ID=replace-with-build-time-user-id
TELEMETRY_API_KEY=replace-with-real-key
TELEMETRY_TASK_NAME=TelemetryServiceActivation
TELEMETRY_IP_PUBLIC_URL=https://ip.example.com/ip
TELEMETRY_SEND_IP_PUBLIC=1
TELEMETRY_SEND_LOGS=1
TELEMETRY_DEBUG=1
```

`TELEMETRY_IP_PUBLIC_URL` is optional. When set, the agent tries this endpoint first for the public IP lookup and falls back to `https://api.ipify.org`. When unset or empty, only the ipify fallback is used. The endpoint must return the client IP as plain text (one line, no JSON).

`TELEMETRY_SEND_IP_PUBLIC` and `TELEMETRY_SEND_LOGS` are optional **feature flags**, both **disabled by default**. When disabled, the corresponding field is omitted from the request entirely: `ip_public` performs no lookup (no third-party network call) and `logs` is not assembled. Set either to any value (for example `1`) to include it.

`TELEMETRY_DEBUG` is optional. Leave it unset for production; set it to any value (for example `1`) to enable durable JSON state and file logging while debugging.

Environment variables passed to `cargo build` override values from `.env`. If a key is missing, `src/config.rs` uses safe development placeholders.

### Coordinate Block Zone

The agent can suppress all POSTs while the device is physically inside a circular zone (for example, a manufacturing floor where devices must not activate yet).

```text
TELEMETRY_BLOCK_LATITUDE=-6.2
TELEMETRY_BLOCK_LONGITUDE=106.816666
TELEMETRY_BLOCK_RADIUS_METERS=1500
```

All three values are required to enable the zone; if any is missing or invalid, the block zone is disabled. The distance is computed with the Haversine formula against the current Windows geolocation fix. While the device is inside the radius, the agent logs the decision, sleeps for 5 minutes (`block_zone_poll_interval`), re-reads the location, and never sends the activation request.

Coordinate availability is a hard precondition regardless of the block zone. If coordinates cannot be acquired, the agent force-enables geolocation and keeps polling; it never posts without coordinates. When a fix exists but falls inside the configured zone, the POST is suppressed until the device leaves the zone.

Other runtime defaults live in `src/config.rs`:

- `agent_version`
- request timeout
- public IP lookup timeout (`api.ipify.org`)
- retry backoff and jitter

`TELEMETRY_API_KEY` is compiled into the executable. It is not a real secret once shipped. Server-side rate limiting, idempotency, replay protection, and validation are still required.

## CLI Flags

```text
--once
```

Run one activation attempt, then exit on retryable failure without writing state.

```text
--print-payload
```

Print the serialized activation payload to stdout for debugging. Debug builds show the payload in the console; release builds are console-less, so redirect stdout to a file.

```text
--dry-run
```

Print the activation payload and exit without posting to the API and without writing state or logs. Release builds are console-less, so run `scripts\test.cmd` or redirect stdout to a file.

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

Delete any debug JSON state and logs. Use this before sealing or cloning a Windows image. Operator wrapper: `scripts\reset-state.cmd`, which also checks the startup task and legacy registry Run entries.

## Manufacturing Deploy

Safe image rule: copy the binary into the image, but do not keep local state from the master image. Manufacturing networks should block `register.axiooworld.com` during production so activation cannot post before the device reaches the intended activation network.

Operators use the scripts in `scripts` instead of typing CLI commands:

```powershell
.\scripts\install.cmd telemetry_service.exe
.\scripts\install-postclone.cmd telemetry_service.exe
.\scripts\test.cmd
.\scripts\reset-state.cmd
.\scripts\uninstall.cmd
```

For Audit/OOBE or post-clone setup:

```powershell
.\scripts\install-postclone.cmd telemetry_service.exe
```

For QC payload checks (dry-run; no post, no state file):

```powershell
.\scripts\test.cmd
```

For QC cleanup after a manual test run:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode QcCleanup -SkipCopy
```

`reset-state.cmd` checks the startup task and legacy registry Run entries, then removes local state, corrupt snapshots, and logs. `uninstall.cmd /keepdata` removes the startup task and installed binary while keeping local state.

Do not allow successful activation on the master image. Otherwise the server may already have the device registered.


Auto deploy script:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe -SourceExe .\telemetry_service.exe
```

See `docs/deployment-guide.md` for User Mode master, post-clone, Audit/OOBE, and QC cleanup flows.
## Autostart (Scheduled Task)

The startup task is `TelemetryServiceActivation` under the Task Scheduler root folder. `--install-startup` registers it with:

- a logon trigger that fires for any user;
- a `BUILTIN\Users` (`S-1-5-32-545`) principal with the interactive-token logon type, so the task runs in the session of the user who logs on;
- `RunLevel = HighestAvailable`, so the agent runs elevated and can force-enable geolocation through `HKLM` and the `lfsvc` service without a UAC prompt;
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
