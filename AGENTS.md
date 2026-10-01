# Memory

## Project Overview
See @README.md for project overview and @package.json for available npm/pnpm commands for this project.

## Code Style Guidelines
- Use descriptive variable names
- Follow existing patterns in the codebase
- Extract complex conditions into meaningful boolean variables

## Architecture Notes

### Geolocation is a hard requirement
The activation POST is only sent once real Windows geolocation coordinates exist
and the device is outside any configured block zone. `wait_for_usable_location`
in `src/main.rs` force-enables geolocation first (`ensure_location_enabled` in
`src/location.rs` writes the HKLM consent store `Value=Allow` and starts the
`lfsvc` service), then polls until a fix is available. Devices without a usable
fix never post; the loop retries forever by design.

This is why the Scheduled Task principal now uses `TASK_RUNLEVEL_HIGHEST`
(`src/autostart.rs`): forcing the HKLM consent value and the system service
needs an elevated token, and it must happen silently without a UAC prompt.

### Activation log payload
Production writes no log files. `src/logging.rs` keeps an in-memory ring buffer
of structured tracing events (last `MAX_LOG_RECORDS`, currently 200) and ships
them in the `logs` multipart field as a JSON array. The buffer is snapshotted
right before each POST so retries carry previous attempt logs. Debug mode also
writes the same events to `activation.log`.

### Payload fields
`DeviceRegistration` posts `serial_number`, `latitude`, `longitude`, and
`accuracy_meters` always. `ip_public` and `logs` are **opt-in build-time
feature flags** (`TELEMETRY_SEND_IP_PUBLIC`, `TELEMETRY_SEND_LOGS` in
`src/config.rs`), both disabled by default. When disabled the field is left out
of the multipart form, and `ip_public` also skips its lookup entirely so no
third-party network call happens. When enabled, `ip_public` resolves in
`src/ip_public.rs` (endpoint from `TELEMETRY_IP_PUBLIC_URL` first, then
`https://api.ipify.org`) and failures fall back to text `null` rather than
aborting activation.


## Common Workflows
Document frequently used workflows and commands here.
