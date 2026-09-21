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

## CLI Deploy (referensi)

Operator tidak perlu mengetik command ini. Semua langkah di guide ini memakai script di section **Script Deploy**. Command berikut hanya referensi perilaku binary:

```powershell
& "C:\Program Files\TelemetryService\telemetry_service.exe" --reset-state
& "C:\Program Files\TelemetryService\telemetry_service.exe" --install-startup
& "C:\Program Files\TelemetryService\telemetry_service.exe" --remove-startup
```

Command behavior:

- `--reset-state`: hapus local activation state dan logs.
- `--install-startup`: register Scheduled Task `TelemetryServiceActivation` yang jalan saat logon user mana pun, di session user tersebut. Wajib elevated karena logon-trigger task butuh hak administrator. Task diberi security descriptor yang mengizinkan `Authenticated Users` menghapus task, sehingga agent non-elevated bisa self-delete setelah aktivasi sukses tanpa UAC prompt. Folder install dan binary juga diberi hak delete-only untuk `BUILTIN\Users`, supaya agent bisa menghapus aplikasinya sendiri. Command ini juga menghapus legacy registry `Run` entry `TelemetryServiceActivation` di `HKLM` dan `HKCU`.
- `--remove-startup`: hapus Scheduled Task `TelemetryServiceActivation`; task tidak ada dianggap sukses. Legacy registry `Run` entry ikut dihapus.

## Script Deploy

Semua langkah install dan cleanup di guide ini memakai script di folder `scripts`. Script `.cmd` otomatis meminta elevasi UAC, jadi operator tidak perlu mengetik command CLI satu per satu.

| Script | Fungsi |
| --- | --- |
| `scripts\install.cmd [exe]` | Master User Mode: copy binary, remove startup task, reset state. |
| `scripts\install-postclone.cmd [exe]` | Mesin final post-clone: copy binary jika ada sumber, reset state, install startup task. |
| `scripts\test.cmd [exe]` | Dry-run payload: tidak post ke API dan tidak menulis state. |
| `scripts\reset-state.cmd [exe]` | Cek Scheduled Task dan legacy registry Run entry, lalu reset state dan logs. |
| `scripts\uninstall.cmd [/keepdata]` | Hapus startup task, state/logs, dan binary. `/keepdata` menyimpan state dan logs. |

Contoh pemakaian:

```powershell
.\scripts\install.cmd telemetry_service.exe
.\scripts\install-postclone.cmd telemetry_service.exe
.\scripts\test.cmd
.\scripts\reset-state.cmd
.\scripts\uninstall.cmd
```

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

Di image master User Mode, jalankan satu script (auto-elevate):

```powershell
.\scripts\install.cmd telemetry_service.exe
```

Script ini copy binary ke `C:\Program Files\TelemetryService`, remove startup task, dan reset state. Jangan install startup task aktif di master sebelum clone, kecuali yakin agent tidak akan jalan.

Setelah clone masuk mesin final, jalankan script post-clone:

```powershell
.\scripts\install-postclone.cmd telemetry_service.exe
```

Jika binary sudah ada di `C:\Program Files\TelemetryService` dan tidak ada sumber exe di folder script, script tetap jalan memakai binary terpasang, reset state, lalu install startup task.

Saat user pertama login, Scheduled Task menjalankan agent. Agent akan:

1. membuat state baru di memory;
2. collect hardware dan lokasi;
3. kirim aktivasi;
4. retry jika offline/server belum tersedia tanpa menulis local state;
5. buat `activation_state.json` hanya setelah API sukses (`result = 0`);
6. hapus startup task;
7. exit.

### QC Test Di Master User Mode

Jika operator harus test agent di master, jalankan dry-run lewat script (tidak post ke API dan tidak menulis state):

```powershell
.\scripts\test.cmd
.\scripts\reset-state.cmd
```

`reset-state.cmd` mengecek status startup task dan legacy Run entry, lalu memastikan state dan logs bersih. Setelah itu baru clone.

### Pre-Clone Checklist

Run sebelum capture/clone:

```powershell
.\scripts\reset-state.cmd
```

Expected result:

```text
[reset] startup task not found: TelemetryServiceActivation
[reset] done; local activation state is clear
```

Jika script melaporkan startup task masih ada, atau state/logs gagal dihapus (exit code bukan 0), jangan capture/clone image.

## Workflow 2 — OOBE Mode Dengan QC Di Audit Mode

Ini workflow paling aman untuk manufaktur. Audit Mode dipakai untuk install binary dan QC, lalu image dikembalikan ke OOBE untuk end user.

### Recommended Flow In Audit Mode

Prepare binary, state bersih, dan startup task dengan satu script dari elevated PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe -SourceExe .\telemetry_service.exe
```

Mode `AuditOobe` copy binary ke Program Files, reset state, lalu install startup task. Alternatif lain: `.\scripts\install-postclone.cmd telemetry_service.exe`.

Lalu seal ke OOBE:

```powershell
sysprep /oobe /shutdown
```

Saat user pertama login setelah OOBE, Scheduled Task menjalankan agent dan aktivasi dimulai.

### QC Test Di Audit Mode

Jika QC perlu memastikan payload, WMI, lokasi, dan HTTP classification berjalan tanpa post ke API:

```powershell
.\scripts\test.cmd
```

Setelah QC selesai, bersihkan state dan install ulang startup task dengan satu script:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode QcCleanup -SkipCopy
```

Mode `QcCleanup` remove startup task, reset state, lalu install startup task kembali. Tanpa `-SkipCopy`, script juga menyalin binary dari `-SourceExe`.

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

`--install-startup` (elevated) melakukan tiga hal:

1. Mendaftarkan Scheduled Task `TelemetryServiceActivation`:
   - logon trigger untuk user mana pun;
   - principal `BUILTIN\Users` (`S-1-5-32-545`) dengan interactive-token logon type, sehingga task jalan di session user yang login;
   - security descriptor yang memberi `Authenticated Users` hak read/execute plus `DELETE` pada task;
   - action dijalankan dengan argumen `--self-delete-on-success`.
2. Memberi `BUILTIN\Users` hak **delete-only** (`D,DC`) pada folder `C:\Program Files\TelemetryService` dan isinya via `icacls`. Hanya delete, bukan write, jadi user bisa menghapus binary tetapi tidak bisa menggantinya.
3. Menghapus legacy registry `Run` entry.

Saat aktivasi sukses, agent:
- menyimpan `activation_state.json`;
- menghapus Scheduled Task-nya sendiri (punya hak `DELETE`);
- menjalankan helper tersembunyi (`cmd` tanpa window) yang menunggu proses agent exit lalu menghapus `telemetry_service.exe` dan foldernya;
- exit.

Tidak ada UAC prompt, tidak ada startup entry tersisa, dan binary ikut bersih. Task dan ACL tidak auto-delete sebelum aktivasi sukses, jadi retry lintas reboot tetap aman.

Install wajib elevated karena logon-trigger task bersifat privileged; agent runtime sendiri selalu non-elevated.

**Penting:** penghapusan binary bersifat **one-way**. Setelah sukses, unit tidak bisa aktivasi ulang, QC ulang, atau refurb tanpa menyalin binary lagi. Kalau itu tidak diinginkan, jangan pakai `--self-delete-on-success` (cukup andalkan self-delete task).

Verify task, security descriptor, dan ACL:

```powershell
schtasks /Query /TN "TelemetryServiceActivation" /V /FO LIST
$svc = New-Object -ComObject Schedule.Service; $svc.Connect()
$svc.GetFolder('\').GetTask('TelemetryServiceActivation').GetSecurityDescriptor(0xF)
icacls "C:\Program Files\TelemetryService"
```

### Hasil Akhir Setelah Aktivasi Sukses

| Item | Status |
| --- | --- |
| Scheduled Task autostart | terhapus |
| `C:\Program Files\TelemetryService\telemetry_service.exe` | terhapus (helper) |
| Folder `C:\Program Files\TelemetryService` | terhapus (helper) |
| Registry `Run` legacy | tidak ada |
| `%ProgramData%\TelemetryService\activation_state.json` dan logs | tetap (audit) |

### Residual Task Setelah Aktivasi

Jika self-delete task gagal (misal ACL task diubah policy), task yang tersisa hanya membuat agent jalan ~50ms lalu exit di setiap login berikutnya (`activated = true`). State tetap di `%ProgramData%`, jadi user baru tidak mengulang aktivasi, dan helper app-removal akan dicoba lagi saat itu. Jalankan `.\scripts\reset-state.cmd` (cek task + reset state) atau `.\scripts\uninstall.cmd /keepdata` dari elevated session untuk membersihkan manual.

Untuk unit yang di-refurbish/QC ulang, siapkan ulang dari source binary v0.3.0:

```powershell
.\scripts\install.cmd telemetry_service.exe
.\scripts\install-postclone.cmd
```

`install.cmd` mengembalikan binary ke Program Files, remove startup task, dan reset state. Setelah QC selesai, `install-postclone.cmd` memasang startup task kembali.

## Troubleshooting

Check startup task:

```powershell
schtasks /Query /TN "TelemetryServiceActivation" /V /FO LIST
```

Run once manually (dry-run, tidak post):

```powershell
.\scripts\test.cmd
```

Untuk benar-benar post satu kali ke API, gunakan `.\scripts\post-once.cmd` (bisa menulis state jika sukses).

Check state:

```powershell
Get-Content "C:\ProgramData\TelemetryService\activation_state.json"
```

Check logs:

```powershell
Get-Content "C:\ProgramData\TelemetryService\logs\activation.log"
```

Reset local activation data (sekaligus cek startup task dan legacy Run entry):

```powershell
.\scripts\reset-state.cmd
```

Reinstall startup task:

```powershell
.\scripts\install-postclone.cmd
```

Uninstall total (startup task + state + binary):

```powershell
.\scripts\uninstall.cmd
```

## Operator Rules

- Jalankan script di folder `scripts`; operator tidak perlu mengetik command CLI manual.
- QC payload pakai `.\scripts\test.cmd` (dry-run). Jangan pakai `--once` untuk QC karena ikut post ke API.
- Jangan clone image setelah agent berhasil aktivasi.
- Jangan clone image yang punya `activation_state.json`.
- Setelah test manual di master/Audit Mode, selalu run `.\scripts\reset-state.cmd`.
- Untuk User Mode clone, install startup task aktif sebaiknya dilakukan post-clone.
- Untuk OOBE/Audit Mode, install startup task sebelum `sysprep /oobe /shutdown` aman selama state sudah di-reset.
- Binary ikut terhapus setelah aktivasi sukses (one-way). Pastikan unit sudah tidak butuh QC/refurb sebelum aktivasi berjalan, atau simpan salinan binary untuk QC ulang.
- Jangan kirim `.env` ke unit produksi; hanya `.exe` yang dibutuhkan.
