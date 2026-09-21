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
- `--install-startup`: register Scheduled Task `TelemetryServiceActivation` yang jalan saat logon user mana pun, di session user tersebut. Wajib elevated karena logon-trigger task butuh hak administrator. Task diberi security descriptor yang mengizinkan `Authenticated Users` menghapus task, sehingga agent non-elevated bisa self-delete setelah aktivasi sukses tanpa UAC prompt. Command ini juga menghapus legacy registry `Run` entry `TelemetryServiceActivation` di `HKLM` dan `HKCU`.
- `--remove-startup`: hapus Scheduled Task `TelemetryServiceActivation`; task tidak ada dianggap sukses. Legacy registry `Run` entry ikut dihapus.

## Auto Deploy Script

Gunakan script ini dari elevated PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe
```

Mode yang tersedia:

- `UserModeMaster`: copy binary, remove startup task, reset state; aman untuk master sebelum clone.
- `PostClone`: copy binary, reset state, install startup task; dipakai di mesin final hasil clone.
- `AuditOobe`: copy binary, reset state, install startup task; dipakai di Audit Mode sebelum `sysprep /oobe /shutdown`.
- `QcCleanup`: copy binary, remove startup task, reset state, install startup task; dipakai setelah QC test.
- `InstallOnly`: copy binary dan install startup task.
- `RemoveOnly`: remove startup task saja.

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

Risiko utama: Windows sudah login dan startup task bisa menjalankan agent sebelum image dikloning. Kalau agent sempat run di master, state akan dibuat di master.

### Recommended Flow

Di image master User Mode:

```powershell
New-Item -ItemType Directory -Force "C:\Program Files\TelemetryService"
Copy-Item "telemetry_service.exe" "C:\Program Files\TelemetryService\telemetry_service.exe" -Force
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
```

Jangan install startup task aktif di master sebelum clone, kecuali yakin agent tidak akan jalan.

Setelah clone masuk mesin final, jalankan first-boot/post-clone script:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

Saat user pertama login, Scheduled Task menjalankan agent. Agent akan:

1. membuat state baru di memory;
2. collect hardware dan lokasi;
3. kirim aktivasi;
4. retry jika offline/server belum tersedia tanpa menulis local state;
5. buat `activation_state.json` hanya setelah API sukses (`result = 0`);
6. hapus startup task;
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

Jika QC tidak perlu menjalankan activation agent, langsung prepare startup task:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

Lalu seal ke OOBE:

```powershell
sysprep /oobe /shutdown
```

Saat user pertama login setelah OOBE, Scheduled Task menjalankan agent dan aktivasi dimulai.

### QC Test Di Audit Mode

Jika QC perlu memastikan payload, WMI, lokasi, dan HTTP classification berjalan:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --once --print-payload
```

Setelah QC selesai, reset state lalu install ulang startup task:

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
schtasks /Query /TN "TelemetryServiceActivation"
```

Expected:

```text
True
False
Task exists
```

## Scheduled Task dan Self-Delete Tanpa Admin

`--install-startup` (elevated) mendaftarkan Scheduled Task `TelemetryServiceActivation` dengan:

- logon trigger untuk user mana pun;
- principal `BUILTIN\Users` (`S-1-5-32-545`) dengan interactive-token logon type, sehingga task jalan di session user yang login;
- security descriptor yang memberi `Authenticated Users` hak read/execute plus `DELETE` pada task.

Karena ada hak `DELETE`, agent yang jalan non-elevated bisa menghapus task-nya sendiri setelah aktivasi sukses. Tidak ada UAC prompt dan tidak ada startup entry yang tertinggal. Task tidak auto-delete sebelum aktivasi sukses, jadi retry lintas reboot tetap aman.

Install wajib elevated karena logon-trigger task bersifat privileged; agent runtime sendiri selalu non-elevated.

Verify task dan security descriptor-nya:

```powershell
schtasks /Query /TN "TelemetryServiceActivation" /V /FO LIST
$svc = New-Object -ComObject Schedule.Service; $svc.Connect()
$svc.GetFolder('\').GetTask('TelemetryServiceActivation').GetSecurityDescriptor(0xF)
```

### Residual Task Setelah Aktivasi

Jika karena suatu hal self-delete gagal (misal ACL task diubah policy), task yang tersisa hanya membuat agent jalan ~50ms lalu exit di setiap login berikutnya (`activated = true`). State tetap di `%ProgramData%`, jadi user baru tidak mengulang aktivasi. Jalankan `--remove-startup` dari elevated session untuk membersihkan manual.

Untuk unit yang di-refurbish/QC ulang, jalankan:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
```

## Troubleshooting

Check startup task:

```powershell
schtasks /Query /TN "TelemetryServiceActivation" /V /FO LIST
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

Reinstall startup task:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
```

## Operator Rules

- Jangan clone image setelah agent berhasil aktivasi.
- Jangan clone image yang punya `activation_state.json`.
- Setelah test manual di master/Audit Mode, selalu run `--reset-state`.
- Untuk User Mode clone, install startup task aktif sebaiknya dilakukan post-clone.
- Untuk OOBE/Audit Mode, install startup task sebelum `sysprep /oobe /shutdown` aman selama state sudah di-reset.
- Jangan kirim `.env` ke unit produksi; hanya `.exe` yang dibutuhkan.
