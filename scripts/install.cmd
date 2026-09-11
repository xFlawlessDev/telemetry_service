@echo off
:: Telemetry Service - User Mode master install (deployment-guide.md Workflow 1)
::
:: Usage:
::   install.cmd [path\to\telemetry_service.exe]
::
:: Default source: telemetry_service.exe next to this script.
:: Copies the binary to Program Files, removes the startup entry, and resets
:: activation state/logs. Safe to run before cloning the master image.

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

    if not exist "%SOURCE_EXE%" (
        echo [install] source executable not found: %SOURCE_EXE%
        exit /B 1
    )

    if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
    copy /Y "%SOURCE_EXE%" "%INSTALL_EXE%" >nul
    if errorlevel 1 (
        echo [install] failed to copy binary to %INSTALL_EXE%
        exit /B 1
    )
    echo [install] copied binary to %INSTALL_EXE%

    "%INSTALL_EXE%" --remove-startup
    if errorlevel 1 (
        echo [install] --remove-startup failed
        exit /B 1
    )

    "%INSTALL_EXE%" --reset-state
    if errorlevel 1 (
        echo [install] --reset-state failed
        exit /B 1
    )

    echo [install] master prepared; do not install startup entry until post-clone
    echo [install] done
    exit /B 0
