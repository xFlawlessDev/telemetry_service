@echo off
:: Telemetry Service - full uninstall (startup entry + state + installed files)
::
:: Usage:
::   uninstall.cmd [/keepdata]
::
:: Removes the registry Run startup entry (HKLM + HKCU + StartupApproved),
:: the legacy scheduled task, activation state/logs in ProgramData, and the
:: installed directory in Program Files. Safe to run when nothing is installed.
:: /keepdata keeps activation state and logs in %ProgramData%\TelemetryService.
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

    set "KEEP_DATA="
    if /I "%~1"=="/keepdata" set "KEEP_DATA=1"

    set "ENTRY_NAME=TelemetryServiceActivation"
    set "INSTALL_DIR=C:\Program Files\TelemetryService"
    set "INSTALL_EXE=%INSTALL_DIR%\telemetry_service.exe"
    set "DATA_DIR=%ProgramData%\TelemetryService"

    echo [uninstall] stopping agent if running
    taskkill /IM telemetry_service.exe /F >nul 2>&1

    if exist "%INSTALL_EXE%" (
        echo [uninstall] unregistering startup entry via agent
        "%INSTALL_EXE%" --remove-startup
        if errorlevel 1 echo [uninstall] --remove-startup reported an error; continuing with registry cleanup

        if not defined KEEP_DATA (
            echo [uninstall] resetting activation state and logs
            "%INSTALL_EXE%" --reset-state
            if errorlevel 1 echo [uninstall] --reset-state reported an error; continuing with directory cleanup
        )
    ) else (
        echo [uninstall] installed executable not found: %INSTALL_EXE%
    )

    echo [uninstall] removing startup registry entries
    reg delete "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" /v "%ENTRY_NAME%" /f >nul 2>&1
    reg delete "HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run" /v "%ENTRY_NAME%" /f >nul 2>&1
    reg delete "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" /v "%ENTRY_NAME%" /f >nul 2>&1
    reg delete "HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" /v "%ENTRY_NAME%" /f >nul 2>&1

    echo [uninstall] removing legacy scheduled task
    schtasks /Delete /TN "%ENTRY_NAME%" /F >nul 2>&1

    if defined KEEP_DATA (
        echo [uninstall] keeping state and logs at %DATA_DIR%
    ) else (
        if exist "%DATA_DIR%" (
            echo [uninstall] removing state and logs at %DATA_DIR%
            rmdir /S /Q "%DATA_DIR%"
            if exist "%DATA_DIR%" echo [uninstall] failed to fully remove %DATA_DIR%
        )
    )

    if exist "%INSTALL_DIR%" (
        echo [uninstall] removing installed files at %INSTALL_DIR%
        rmdir /S /Q "%INSTALL_DIR%"
        if exist "%INSTALL_DIR%" (
            echo [uninstall] failed to remove %INSTALL_DIR%; close any process using it and retry
            exit /B 1
        )
    )

    echo [uninstall] done
    pause
    exit /B 0
