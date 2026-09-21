@echo off
:: Telemetry Service - startup task check + activation state reset
::
:: Usage:
::   reset-state.cmd [path\to\telemetry_service.exe]
::
:: Reports the Scheduled Task and legacy registry Run startup entries, then
:: stops the agent and removes activation state, corrupt snapshots, and logs
:: from %ProgramData%\TelemetryService (plus the %LOCALAPPDATA% fallback).
:: Use before sealing/cloning a master image or for QC cleanup. Safe to run
:: when nothing is installed.
::
:: ENTRY_NAME must match TELEMETRY_TASK_NAME if the binary was built with it.

:: Check for administrative privileges
net session >nul 2>&1
if %errorLevel% == 0 (
    goto :init
) else (
    goto :UACPrompt
)

:UACPrompt
    echo Set UAC = CreateObject^("Shell.Application"^) > "%temp%\getadmin.vbs"
    echo UAC.ShellExecute "cmd.exe", "/c ""%~s0"" %*", "", "runas", 1 >> "%temp%\getadmin.vbs"
    "%temp%\getadmin.vbs"
    del "%temp%\getadmin.vbs"
    exit /B

:init
    :: Change directory to the script's actual location
    cd /d "%~dp0"

    setlocal EnableExtensions

    set "SOURCE_EXE=%~dp0telemetry_service.exe"
    if not "%~1"=="" set "SOURCE_EXE=%~1"
    set "INSTALL_DIR=C:\Program Files\TelemetryService"
    set "INSTALL_EXE=%INSTALL_DIR%\telemetry_service.exe"
    set "DATA_DIR=%ProgramData%\TelemetryService"
    set "FALLBACK_DATA_DIR=%LOCALAPPDATA%\TelemetryService"
    set "ENTRY_NAME=TelemetryServiceActivation"

    set "STATE_FOUND="

    echo [reset] checking startup task
    schtasks /Query /TN "%ENTRY_NAME%" >nul 2>&1
    if errorlevel 1 (
        echo [reset] startup task not found: %ENTRY_NAME%
    ) else (
        echo [reset] startup task exists: %ENTRY_NAME%
        schtasks /Query /TN "%ENTRY_NAME%" /FO TABLE
        echo [reset] warning: while the task exists, the next logon can activate this device again
        echo [reset] run "telemetry_service.exe --remove-startup" first on a master image
    )

    echo [reset] checking legacy registry Run entries
    reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" /v "%ENTRY_NAME%" >nul 2>&1
    if not errorlevel 1 echo [reset] legacy Run entry found in HKLM
    reg query "HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" /v "%ENTRY_NAME%" >nul 2>&1
    if not errorlevel 1 echo [reset] legacy Run entry found in HKCU

    echo [reset] stopping agent if running
    taskkill /IM telemetry_service.exe /F >nul 2>&1

    set "TARGET_EXE=%INSTALL_EXE%"
    if not exist "%TARGET_EXE%" set "TARGET_EXE=%SOURCE_EXE%"

    if exist "%DATA_DIR%\activation_state.json" set "STATE_FOUND=1"
    if exist "%DATA_DIR%\logs" set "STATE_FOUND=1"

    if exist "%TARGET_EXE%" (
        echo [reset] resetting state via "%TARGET_EXE%" --reset-state
        "%TARGET_EXE%" --reset-state
        if errorlevel 1 echo [reset] --reset-state reported an error; falling back to direct removal
    ) else (
        echo [reset] agent executable not found; removing state directly
    )

    echo [reset] removing leftover state, corrupt snapshots, and logs
    if exist "%DATA_DIR%\activation_state.json" del /F /Q "%DATA_DIR%\activation_state.json"
    if exist "%DATA_DIR%\activation_state.json.corrupt.*" del /F /Q "%DATA_DIR%\activation_state.json.corrupt.*"
    if exist "%DATA_DIR%\logs" rmdir /S /Q "%DATA_DIR%\logs"
    if exist "%DATA_DIR%" rmdir "%DATA_DIR%" 2>nul

    if exist "%FALLBACK_DATA_DIR%\activation_state.json" (
        echo [reset] removing fallback state at %FALLBACK_DATA_DIR%
        del /F /Q "%FALLBACK_DATA_DIR%\activation_state.json"
    )
    if exist "%FALLBACK_DATA_DIR%\activation_state.json.corrupt.*" del /F /Q "%FALLBACK_DATA_DIR%\activation_state.json.corrupt.*"
    if exist "%FALLBACK_DATA_DIR%\logs" rmdir /S /Q "%FALLBACK_DATA_DIR%\logs"
    if exist "%FALLBACK_DATA_DIR%" rmdir "%FALLBACK_DATA_DIR%" 2>nul

    if not defined STATE_FOUND echo [reset] no local state or logs were present

    if exist "%DATA_DIR%\activation_state.json" (
        echo [reset] failed to remove %DATA_DIR%\activation_state.json
        exit /B 1
    )
    if exist "%DATA_DIR%\logs" (
        echo [reset] failed to remove %DATA_DIR%\logs
        exit /B 1
    )

    echo [reset] done; local activation state is clear
    pause
    exit /B 0
