@echo off
:: Telemetry Service - PostClone install (deployment-guide.md Workflow 1)
::
:: Usage:
::   install-postclone.cmd [path\to\telemetry_service.exe]
::
:: Run on the final cloned machine. Copies the binary if a source exists,
:: resets activation state/logs, then installs the registry Run startup entry
:: so activation runs at first user login.

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

    set "SOURCE_EXE=%~dp0telemetry_service.exe"
    if not "%~1"=="" set "SOURCE_EXE=%~1"
    set "INSTALL_DIR=C:\Program Files\TelemetryService"
    set "INSTALL_EXE=%INSTALL_DIR%\telemetry_service.exe"

    if exist "%SOURCE_EXE%" (
        if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
        copy /Y "%SOURCE_EXE%" "%INSTALL_EXE%" >nul
        if errorlevel 1 (
            echo [postclone] failed to copy binary to %INSTALL_EXE%
            exit /B 1
        )
        echo [postclone] copied binary to %INSTALL_EXE%
    ) else (
        echo [postclone] source executable not found: %SOURCE_EXE%
        echo [postclone] skipping copy, using installed binary
    )

    if not exist "%INSTALL_EXE%" (
        echo [postclone] installed executable not found: %INSTALL_EXE%
        exit /B 1
    )

    "%INSTALL_EXE%" --reset-state
    if errorlevel 1 (
        echo [postclone] --reset-state failed
        exit /B 1
    )

    "%INSTALL_EXE%" --install-startup
    if errorlevel 1 (
        echo [postclone] --install-startup failed
        exit /B 1
    )

    echo [postclone] activation startup entry installed; agent runs at first user login
    echo [postclone] done
    exit /B 0
