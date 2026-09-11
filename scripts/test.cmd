@echo off
:: Telemetry Service - payload dry-run viewer (deployment-guide.md QC checks)
::
:: Usage:
::   test.cmd [path\to\telemetry_service.exe]
::
:: Prints the activation payload without posting it to the API and without
:: writing activation state. Requires a binary built with --dry-run support.
:: Default target: installed binary, falling back to telemetry_service.exe
:: next to this script.

setlocal EnableExtensions

set "SOURCE_EXE=%~dp0telemetry_service.exe"
if not "%~1"=="" set "SOURCE_EXE=%~1"
set "INSTALL_EXE=C:\Program Files\TelemetryService\telemetry_service.exe"

set "TARGET_EXE=%INSTALL_EXE%"
if not exist "%TARGET_EXE%" set "TARGET_EXE=%SOURCE_EXE%"

if not exist "%TARGET_EXE%" (
    echo [test] telemetry_service.exe not found.
    echo [test] Usage: test.cmd [path\to\telemetry_service.exe]
    exit /B 1
)

set "PAYLOAD_FILE=%TEMP%\telemetry_payload_%RANDOM%.txt"
"%TARGET_EXE%" --dry-run > "%PAYLOAD_FILE%" 2>&1
set "EXIT_CODE=%errorLevel%"

findstr /C:"dry-run" "%PAYLOAD_FILE%" >nul
if errorlevel 1 (
    echo [test] binary does not support --dry-run; rebuild and redeploy it
    del "%PAYLOAD_FILE%" >nul 2>&1
    exit /B 1
)

type "%PAYLOAD_FILE%"
del "%PAYLOAD_FILE%" >nul 2>&1

if not "%EXIT_CODE%"=="0" (
    echo [test] dry-run failed with exit code %EXIT_CODE%
    exit /B %EXIT_CODE%
)

echo [test] dry-run only; nothing was posted to the API
exit /B 0
