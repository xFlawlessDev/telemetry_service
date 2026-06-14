# Telemetry Service Activation Agent Plan

## Target desain

Aplikasi Rust background agent yang:

1. auto-start saat Windows boot/login;
2. collect device identity lewat WMI;
3. collect lokasi lewat `windows::Devices::Geolocation`;
4. retry kirim aktivasi sampai internet/server tersedia;
5. setelah sukses, menandai state lokal `activated`;
6. unregister mekanisme autostart;
7. exit bersih.

Self-delete dari proses utama tidak direkomendasikan. Lebih aman: unregister scheduled task lalu biarkan installer/updater cleanup binary.

## Phase 1 — Project foundation

### Files

- `Cargo.toml`
- `src/main.rs`
- `src/error.rs`
- `src/config.rs`
- `src/state.rs`

### Work

- Tambah dependency:
  - `windows`
  - `wmi`
  - `serde`
  - `serde_json`
  - `reqwest`
  - `tokio`
  - `thiserror`
  - `uuid`
  - `rand`
  - `tracing`
  - `tracing-subscriber`
  - `time` atau `chrono`
- Ubah binary jadi async runtime `tokio`.
- Tambah error enum `AppError`.
- Tambah `AppConfig`:
  - `base_url: &'static str`
  - `api_key: &'static str`
  - `agent_version: &'static str`
  - retry/backoff config
  - request timeout
- Tambah `ActivationState`:
  - `install_id`
  - `activated`
  - `activation_id`
  - `attempt_count`
  - `first_seen_utc`
  - `last_attempt_utc`
  - `last_error`

### Acceptance

- `cargo check` pass.
- App bisa start, load/create state file, lalu exit sementara tanpa panic.

## Phase 2 — Local state

### Files

- `src/state.rs`
- `src/paths.rs`

### Work

- Tentukan path state:
  - ideal: `%ProgramData%\TelemetryService\activation_state.json`
  - fallback dev: current dir atau `%LOCALAPPDATA%`
- Implement:
  - `load_or_initialize_state()`
  - `save_state_atomic()`
- Atomic write:
  - tulis ke `.tmp`
  - flush
  - rename ke final path
- Jangan `unwrap`.
- Kalau state corrupt:
  - rename file lama ke `.corrupt.<timestamp>`
  - buat state baru

### Acceptance

- First run membuat state.
- Run berikutnya memakai `install_id` yang sama.
- Jika `activated = true`, app langsung cleanup autostart lalu exit.

## Phase 3 — WMI hardware collector

### Files

- `src/hardware.rs`

### Work

- Query minimal:
  - `Win32_BIOS.SerialNumber`
  - `Win32_ComputerSystemProduct.UUID`
  - `Win32_BaseBoard.SerialNumber`
  - optional: `Win32_ComputerSystem.Manufacturer`, `Model`
- Define structs:
  - `Win32_BIOS`
  - `Win32_ComputerSystemProduct`
  - `Win32_BaseBoard`
  - `Win32_ComputerSystem`
- Normalize invalid values:
  - empty string
  - `"To be filled by O.E.M."`
  - `"Default string"`
  - `"System Serial Number"`
  - all-zero UUID
- Output:
  - `HardwareIdentity`
  - field optional, not fatal unless all identifiers missing

### Acceptance

- WMI error returns `AppError::Wmi`.
- Missing BIOS SN does not crash.
- Payload can still be built with partial hardware identity.

## Phase 4 — Geolocation collector

### Files

- `src/location.rs`

### Work

- Use:
  - `windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator}`
- Implement:
  - `request_access() -> GeolocationAccessStatus`
  - `get_location(timeout) -> LocationSnapshot`
- Model:
  - `LocationSnapshot`
    - `access_status`
    - `latitude: Option<f64>`
    - `longitude: Option<f64>`
    - `accuracy_meters: Option<f64>`
    - `timestamp_utc: Option<String>`
    - `error: Option<String>`
- Location failure is non-fatal.
- Timeout geolocation supaya app tidak hang.

### Acceptance

- Kalau permission denied/unavailable, app tetap kirim activation payload tanpa koordinat.
- Kalau location tersedia, latitude/longitude masuk payload.
- Tidak ada infinite wait pada geolocation API.

## Phase 5 — Activation payload + HTTP client

### Files

- `src/api.rs`
- `src/payload.rs`

### Payload shape

```json
{
  "install_id": "uuid",
  "agent_version": "0.1.0",
  "hardware": {
    "bios_serial": "...",
    "system_uuid": "...",
    "baseboard_serial": "...",
    "manufacturer": "...",
    "model": "..."
  },
  "location": {
    "access_status": "Allowed",
    "latitude": -6.2,
    "longitude": 106.8,
    "accuracy_meters": 100.0,
    "timestamp_utc": "..."
  },
  "attempt": {
    "count": 3,
    "first_seen_utc": "...",
    "last_attempt_utc": "..."
  }
}
```

### HTTP

- `POST {base_url}/device-activations`
- Headers:
  - `Authorization: Bearer {api_key}` atau `X-API-Key`
  - `Content-Type: application/json`
  - `Idempotency-Key: install_id`
- Timeout per request.
- Response:

```json
{
  "status": "activated",
  "activation_id": "..."
}
```

### Error handling

- Network timeout = retryable.
- DNS/connect fail = retryable.
- HTTP 5xx = retryable.
- HTTP 429 = retryable, respect `Retry-After` kalau ada.
- HTTP 400/401/403 = non-retryable config/server error.
- Invalid response JSON = retryable atau fatal tergantung policy. Default plan: retryable dengan backoff panjang.

### Acceptance

- Request body serializes deterministically.
- Success marks `state.activated = true`.
- Duplicate request aman karena `Idempotency-Key`.

## Phase 6 — Retry/backoff loop

### Files

- `src/retry.rs`
- `src/main.rs`

### Work

- Loop:
  1. load state
  2. if activated: cleanup and exit
  3. collect hardware
  4. collect location
  5. send activation
  6. on success: save state, cleanup, exit
  7. on retryable failure: save attempt metadata, sleep
- Backoff:
  - start 15s
  - max 15m atau 30m
  - exponential
  - jitter ±20%
- Untuk scheduled task at startup: agent boleh stay running sampai sukses.
- Untuk scheduled task every 15 min: agent coba sekali lalu exit.
- Default plan: stay running sampai sukses karena requirement adalah polling sampai konek internet.

### Acceptance

- Network unavailable tidak crash.
- Attempt count naik dan tersimpan.
- Sleep tidak busy loop.
- Sukses menghentikan loop.

## Phase 7 — Windows autostart cleanup

### Files

- `src/autostart.rs`

### Recommended autostart mechanism

Scheduled Task, bukan registry Run key.

Task example:

```powershell
schtasks /Create /TN "TelemetryServiceActivation" /SC ONLOGON /RL LIMITED /TR "C:\Program Files\TelemetryService\telemetry_service.exe" /F
```

Atau startup:

```powershell
schtasks /Create /TN "TelemetryServiceActivation" /SC ONSTART /RL HIGHEST /TR "C:\Program Files\TelemetryService\telemetry_service.exe" /F
```

### App cleanup after success

Run:

```text
schtasks /Delete /TN "TelemetryServiceActivation" /F
```

### Work

- Implement `disable_autostart()`:
  - spawn `schtasks`
  - ignore “task not found” as success
  - return error for other failures
- Do not delete binary from inside itself.

### Acceptance

- Setelah activation sukses, scheduled task dihapus.
- Jika task tidak ada, app tetap exit sukses.

## Phase 8 — Logging

### Files

- `src/logging.rs`
- `src/main.rs`

### Work

- Use `tracing`.
- Log ke file:
  - `%ProgramData%\TelemetryService\logs\activation.log`
- Jangan log:
  - full API key
  - precise location kalau tidak perlu
  - raw server auth headers
- Log:
  - startup
  - loaded state
  - WMI success/fail summary
  - geolocation status
  - HTTP status
  - retry delay
  - activation success
  - cleanup result

### Acceptance

- Debugging field failure possible dari log.
- Tidak ada secret di log.

## Phase 9 — Windows subsystem/release behavior

### Files

- `src/main.rs`
- `Cargo.toml`

### Work

- Untuk release background tanpa console:

```rust
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
```

- Dev build tetap punya console.
- Optional CLI:
  - `--once`
  - `--install-task`
  - `--remove-task`
  - `--print-payload`

Kalau ingin minimal, skip CLI dan installer yang handle scheduled task.

### Acceptance

- Debug run tetap terlihat output/log.
- Release tidak muncul console window.

## Phase 10 — Tests

### Files

- unit tests inline atau `tests/`

### Test target

- `state`:
  - creates new state
  - preserves `install_id`
  - handles corrupt JSON
- `hardware`:
  - normalization removes invalid serial values
- `retry`:
  - backoff capped
  - jitter stays within expected bounds
- `payload`:
  - missing location serializes as `null`/absent sesuai design
- `api`:
  - classify status:
    - 200 success
    - 429 retryable
    - 500 retryable
    - 401 fatal

### Acceptance

- `cargo test` pass.
- `cargo clippy --all-targets --all-features -- -D warnings` pass.
- `cargo fmt --check` pass.

## Suggested module layout

```text
src/
  main.rs
  api.rs
  autostart.rs
  config.rs
  error.rs
  hardware.rs
  location.rs
  logging.rs
  paths.rs
  payload.rs
  retry.rs
  state.rs
```

## Important security decisions

### API key

Hardcoded API key tidak boleh dianggap secret.

Server harus tetap pakai:

- rate limit;
- idempotency key;
- device fingerprint validation;
- request timestamp;
- replay protection jika ada signature;
- reject impossible duplicate activations.

### Location

Lokasi optional. Jangan block activation hanya karena Windows location permission denied.

### Self-delete

Tidak direkomendasikan. Pakai:

- unregister scheduled task;
- mark activated state;
- installer/updater cleanup.

Kalau tetap wajib self-delete, approach paling aman adalah helper process/script delayed delete setelah main process exit, tapi ini meningkatkan false-positive AV. Default plan tidak memakai self-delete.

## Implementation order

1. Foundation + error/config/state.
2. Hardware WMI collector.
3. Payload builder.
4. HTTP client.
5. Retry loop.
6. Geolocation collector.
7. Autostart cleanup.
8. Logging.
9. Release subsystem.
10. Tests + clippy/fmt.

Urutan ini membuat app bisa diuji lebih cepat: aktivasi dengan hardware dulu, lalu lokasi ditambahkan sebagai field optional.
