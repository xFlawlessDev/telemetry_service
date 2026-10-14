@echo off
:: Telemetry Service - single real activation attempt (posts to the API)
::
:: Usage:
::   post-once.cmd [path\to\telemetry_service.exe] [nopause] [--dns[=<ip>]]
::
:: Runs the agent with --once --print-payload. Unlike test.cmd, this DOES send a
:: real activation request, so the server may record a new device-on entry. Use
:: it to confirm end-to-end connectivity and to inspect the payload a deployed
:: build actually produces.
::
:: Default target: an explicit path wins, otherwise the installed binary, then
:: telemetry_service.exe next to this script. Pass nopause to run unattended.
::
:: --dns temporarily points the API host at the real server through the hosts
:: file, for networks whose DNS resolves the API host to a local mirror that
:: serves an expired certificate. It needs elevation, so it delegates to
:: post-once-dns.ps1 (one UAC prompt); that script removes the hosts entry again
:: afterwards, always. Pass an explicit address with --dns=<ip> when public DNS
:: is not reachable, otherwise the real address is resolved through 1.1.1.1.
::
:: Exit codes: the agent's own exit code, or 1 when the binary is missing or the
:: DNS override could not be applied. Note that --once exits 0 both when
:: activation succeeds and when the server is unreachable (DNS/TLS/offline),
:: because a retryable failure is reported as a finished attempt. Confirm the
:: record through the get_device_on API endpoint.

setlocal EnableExtensions

:: Capture the script directory before any shift: %~dp0 follows %0, and the
:: argument loop below shifts %0 along with the arguments.
set "SCRIPT_DIR=%~dp0"
set "ALL_ARGS=%*"
set "DNS_REQUESTED="
set "NO_PAUSE="
set "TARGET_ARG="

:parse
if "%~1"=="" goto parsed
set "ARG=%~1"
if /I "%ARG%"=="nopause" set "NO_PAUSE=1"
if /I "%ARG%"=="--dns" set "DNS_REQUESTED=1"
if /I "%ARG:~0,6%"=="--dns=" set "DNS_REQUESTED=1"
if not defined TARGET_ARG if /I not "%ARG%"=="nopause" if /I not "%ARG%"=="--dns" if /I not "%ARG:~0,6%"=="--dns=" set "TARGET_ARG=%ARG%"
shift
goto parse

:parsed
set "TARGET_EXE=%TARGET_ARG%"
if not defined TARGET_EXE set "TARGET_EXE=C:\Program Files\TelemetryService\telemetry_service.exe"
if not defined TARGET_ARG if not exist "%TARGET_EXE%" set "TARGET_EXE=%SCRIPT_DIR%telemetry_service.exe"

if not exist "%TARGET_EXE%" goto missing

for %%F in ("%TARGET_EXE%") do echo [post-once] binary: %%~fF [modified %%~tF]

if defined DNS_REQUESTED goto dns

echo [post-once] sending one activation attempt; the server may record a new entry

set "OUT_FILE=%TEMP%\telemetry_post_once_%RANDOM%.txt"
"%TARGET_EXE%" --once --print-payload > "%OUT_FILE%" 2>&1
set "EXIT_CODE=%errorLevel%"

if exist "%OUT_FILE%" (
    type "%OUT_FILE%"
    del "%OUT_FILE%" >nul 2>&1
)

echo [post-once] agent exit code: %EXIT_CODE%
echo [post-once] --once exits 0 even when the server was unreachable, so confirm
echo [post-once] the new record through the get_device_on API endpoint.

if not "%EXIT_CODE%"=="0" echo [post-once] activation did not finish; see the message above
if "%EXIT_CODE%"=="0" echo [post-once] attempt finished

if not defined NO_PAUSE pause
exit /B %EXIT_CODE%

:dns
echo [post-once] --dns requested: delegating to post-once-dns.ps1 ^(needs elevation^)
powershell -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT_DIR%post-once-dns.ps1" %ALL_ARGS%
set "EXIT_CODE=%errorLevel%"
if not "%EXIT_CODE%"=="0" echo [post-once] DNS-override run exited with %EXIT_CODE%; check the output above
exit /B %EXIT_CODE%

:missing
echo [post-once] telemetry_service.exe not found: %TARGET_EXE%
echo [post-once] Usage: post-once.cmd [path\to\telemetry_service.exe] [nopause] [--dns[=<ip>]]
exit /B 1
