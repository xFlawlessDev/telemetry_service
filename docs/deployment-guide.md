# Telemetry Service Manufacturing Deployment Guide

Guide ini untuk dua workflow image Windows di manufaktur:

1. User Mode siap pakai lalu di-clone.
2. OOBE Mode dengan QC di Audit Mode, lalu deploy ke user dalam OOBE.

## Prinsip Utama

Jangan pernah seal atau clone image yang sudah punya state aktivasi.

State yang wajib kosong sebelum image disegel atau dikloning:

```text
%ProgramData%\TelemetryService\activation_state.json
%ProgramData%\TelemetryService\logs\
```

Jika `activation_state.json` ikut ke image master, semua mesin clone bisa memakai `install_id` yang sama. Server akan melihat beberapa device sebagai instalasi yang sama.

Selama proses manufaktur/produksi, blokir domain `register.axiooworld.com` di network produksi agar unit tidak bisa melakukan post aktivasi sebelum sampai ke network aktivasi yang benar.

File yang boleh ikut image:

```text
C:\Program Files\TelemetryService\telemetry_service.exe
```

File yang tidak wajib ikut image:

```text
.env
```

`.env` hanya dibaca saat `cargo build`. Nilai `TELEMETRY_BASE_URL`, `TELEMETRY_API_KEY`, dan `TELEMETRY_TASK_NAME` sudah tertanam di `.exe` hasil build.

## Build Release

Di mesin build/developer:

```powershell
Copy-Item .env.example .env
notepad .env
cargo build --release
```

Contoh `.env`:

```text
TELEMETRY_BASE_URL=https://activation.example.com
TELEMETRY_USER_ID=replace-with-build-time-user-id
TELEMETRY_API_KEY=replace-with-real-key
TELEMETRY_TASK_NAME=TelemetryServiceActivation
```

Copy hasil build ke paket deploy:

```powershell
New-Item -ItemType Directory -Force "C:\Program Files\TelemetryService"
Copy-Item "target\release\telemetry_service.exe" "C:\Program Files\TelemetryService\telemetry_service.exe" -Force
```

## CLI Deploy

Semua command dijalankan dari binary final:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
```

Command behavior:

- `--reset-state`: hapus local activation state dan logs.
- `--install-startup`: register registry `Run` startup entry untuk path `.exe` saat ini. Saat run elevated, entry ditulis ke `HKLM` (semua user); tanpa elevation fallback ke `HKCU` (user saat ini saja). Command ini juga menghapus legacy Scheduled Task `TelemetryServiceActivation` bila masih ada.
- `--remove-startup`: hapus startup entry; entry tidak ada dianggap sukses. Legacy Scheduled Task ikut dihapus bila ada.

## Auto Deploy Script

Gunakan script ini dari elevated PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe
```

Mode yang tersedia:

- `UserModeMaster`: copy binary, remove startup entry, reset state; aman untuk master sebelum clone.
- `PostClone`: copy binary, reset state, install startup entry; dipakai di mesin final hasil clone.
- `AuditOobe`: copy binary, reset state, install startup entry; dipakai di Audit Mode sebelum `sysprep /oobe /shutdown`.
- `QcCleanup`: copy binary, remove startup entry, reset state, install startup entry; dipakai setelah QC test.
- `InstallOnly`: copy binary dan install startup entry.
- `RemoveOnly`: remove startup entry saja.

Parameter umum:

```powershell
-SourceExe .\telemetry_service.exe
-InstallDir "C:\Program Files\TelemetryService"
-SkipCopy
```

Contoh User Mode master:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode UserModeMaster -SourceExe .\telemetry_service.exe
```

Contoh post-clone:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode PostClone -SkipCopy
```

Contoh Audit/OOBE:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe -SourceExe .\telemetry_service.exe
```

## Workflow 1 — User Mode Siap Pakai Lalu Clone

Risiko utama: Windows sudah login dan startup entry bisa menjalankan agent sebelum image dikloning. Kalau agent sempat run di master, state akan dibuat di master.

### Recommended Flow

Di image master User Mode:

```powershell
New-Item -ItemType Directory -Force "C:\Program Files\TelemetryService"
Copy-Item "telemetry_service.exe" "C:\Program Files\TelemetryService\telemetry_service.exe" -Force
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
```

Jangan install startup entry aktif di master sebelum clone, kecuali yakin agent tidak akan jalan.

Setelah clone masuk mesin final, jalankan first-boot/post-clone script:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

Saat user pertama login, registry `Run` key menjalankan agent. Agent akan:

1. membuat state baru di memory;
2. collect hardware dan lokasi;
3. kirim aktivasi;
4. retry jika offline/server belum tersedia tanpa menulis local state;
5. buat `activation_state.json` hanya setelah API sukses (`result = 0`);
6. hapus startup entry;
7. exit.

### QC Test Di Master User Mode

Jika operator harus test agent di master:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --once --print-payload
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
```

Setelah itu baru clone. Jangan skip `--reset-state`.

### Pre-Clone Checklist

Run sebelum capture/clone:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
Test-Path "C:\ProgramData\TelemetryService\activation_state.json"
```

Expected result:

```text
False
```

## Workflow 2 — OOBE Mode Dengan QC Di Audit Mode

Ini workflow paling aman untuk manufaktur. Audit Mode dipakai untuk install binary dan QC, lalu image dikembalikan ke OOBE untuk end user.

### Recommended Flow In Audit Mode

Install binary:

```powershell
New-Item -ItemType Directory -Force "C:\Program Files\TelemetryService"
Copy-Item "telemetry_service.exe" "C:\Program Files\TelemetryService\telemetry_service.exe" -Force
```

Jika QC tidak perlu menjalankan activation agent, langsung prepare startup entry:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

Lalu seal ke OOBE:

```powershell
sysprep /oobe /shutdown
```

Saat user pertama login setelah OOBE, registry `Run` key menjalankan agent dan aktivasi dimulai.

### QC Test Di Audit Mode

Jika QC perlu memastikan payload, WMI, lokasi, dan HTTP classification berjalan:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --once --print-payload
```

Setelah QC selesai, reset state lalu install ulang startup entry:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

Baru seal:

```powershell
sysprep /oobe /shutdown
```

### Pre-Sysprep Checklist

```powershell
Test-Path "C:\Program Files\TelemetryService\telemetry_service.exe"
Test-Path "C:\ProgramData\TelemetryService\activation_state.json"
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" | Select-Object TelemetryServiceActivation
```

Expected:

```text
True
False
Startup entry exists
```

## Run Key: HKLM vs HKCU

Default CLI `--install-startup` menulis ke `HKLM\...\Run` saat dijalankan elevated (semua user, wajib untuk first-login setelah OOBE). Tanpa elevation, entry otomatis fallback ke `HKCU` (hanya user yang menjalankan install).

Use `HKLM` (default elevated) when:

- aktivasi harus berjalan untuk user pertama setelah OOBE/clone;
- deploy script sudah berjalan elevated (Audit Mode, post-clone admin).

`HKCU` fallback hanya cocok untuk test di mesin developer, karena entry tidak akan berpindah ke user lain.

### Residual Entry Setelah Aktivasi

Agent berjalan non-elevated di session user, jadi penghapusan entry `HKLM` mungkin gagal setelah aktivasi sukses (butuh admin). Ini bukan masalah:

- entry yang tersisa hanya membuat agent jalan ~50ms lalu exit di setiap login berikutnya (`activated = true`);
- state tetap di `%ProgramData%`, jadi user baru tidak mengulang aktivasi;
- entry bersih sendiri saat ada admin login, atau jalankan `--remove-startup` dari elevated session.

Untuk unit yang di-refurbish/QC ulang, jalankan:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
```

## Troubleshooting

Check startup entry:

```powershell
Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" | Select-Object TelemetryServiceActivation
```

Run once manually:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --once --print-payload
```

Check state:

```powershell
Get-Content "C:\ProgramData\TelemetryService\activation_state.json"
```

Check logs:

```powershell
Get-Content "C:\ProgramData\TelemetryService\logs\activation.log"
```

Reset local activation data:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
```

Reinstall startup entry:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

## Operator Rules

- Jangan clone image setelah agent berhasil aktivasi.
- Jangan clone image yang punya `activation_state.json`.
- Setelah test manual di master/Audit Mode, selalu run `--reset-state`.
- Untuk User Mode clone, install startup entry aktif sebaiknya dilakukan post-clone.
- Untuk OOBE/Audit Mode, install startup entry sebelum `sysprep /oobe /shutdown` aman selama state sudah di-reset.
- Jangan kirim `.env` ke unit produksi; hanya `.exe` yang dibutuhkan.
