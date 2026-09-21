<#
.SYNOPSIS
Deploy Telemetry Service activation agent for manufacturing Windows images.

.DESCRIPTION
Copies telemetry_service.exe to Program Files, resets activation state/logs, and manages
the self-deleting Scheduled Task startup entry through the agent CLI. The task runs at
logon of any user and grants Authenticated Users DELETE so the non-elevated agent can
remove it after successful activation without a UAC prompt. Delete-only file permissions
are also granted so the agent removes its own binary and folder after success.

Modes:
- UserModeMaster: prepare a clone master; copy binary, remove startup entry, reset state.
- PostClone: run on final cloned machine; copy binary, reset state, install startup entry.
- AuditOobe: run in Audit Mode before sysprep /oobe /shutdown; copy binary, reset state, install startup entry.
- QcCleanup: run after QC test; remove startup entry, reset state, install startup entry.
- InstallOnly: copy binary and install startup entry.
- RemoveOnly: remove startup entry only.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode AuditOobe

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\scripts\deploy.ps1 -Mode PostClone -SourceExe .\telemetry_service.exe
#>

[CmdletBinding()]
param(
    [ValidateSet('UserModeMaster', 'PostClone', 'AuditOobe', 'QcCleanup', 'InstallOnly', 'RemoveOnly')]
    [string]$Mode = 'AuditOobe',

    [string]$SourceExe = (Join-Path $PSScriptRoot '..\target\release\telemetry_service.exe'),

    [string]$InstallDir = 'C:\Program Files\TelemetryService',

    [string]$ExeName = 'telemetry_service.exe',

    [switch]$SkipCopy,

    [switch]$SkipAdminCheck
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Write-Step {
    param([Parameter(Mandatory)][string]$Message)
    Write-Host "[deploy] $Message"
}

function Resolve-FullPath {
    param([Parameter(Mandatory)][string]$Path)
    $executionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
}

function Copy-AgentBinary {
    if ($SkipCopy) {
        Write-Step "skip copy requested"
        return
    }

    $resolvedSource = Resolve-FullPath $SourceExe
    if (-not (Test-Path -LiteralPath $resolvedSource -PathType Leaf)) {
        throw "Source executable not found: $resolvedSource. Build with 'cargo build --release' or pass -SourceExe."
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $destination = Join-Path $InstallDir $ExeName
    Copy-Item -LiteralPath $resolvedSource -Destination $destination -Force
    Write-Step "copied binary to $destination"
}

function Invoke-Agent {
    param([Parameter(Mandatory)][string[]]$Arguments)

    $exePath = Join-Path $InstallDir $ExeName
    if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
        throw "Installed executable not found: $exePath"
    }

    Write-Step "running $exePath $($Arguments -join ' ')"
    & $exePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Agent command failed with exit code ${LASTEXITCODE}: $($Arguments -join ' ')"
    }
}

function Reset-AgentState {
    Invoke-Agent -Arguments @('--reset-state')
}

function Install-AgentStartup {
    Invoke-Agent -Arguments @('--install-startup')
}

function Remove-AgentStartup {
    Invoke-Agent -Arguments @('--remove-startup')
}

if (-not $SkipAdminCheck -and -not (Test-IsAdministrator)) {
    throw 'Run this script from an elevated PowerShell session, or pass -SkipAdminCheck if your environment grants equivalent rights.'
}

Write-Step "mode: $Mode"

switch ($Mode) {
    'UserModeMaster' {
        Copy-AgentBinary
        Remove-AgentStartup
        Reset-AgentState
        Write-Step 'master prepared; do not install startup entry until post-clone'
    }
    'PostClone' {
        Copy-AgentBinary
        Reset-AgentState
        Install-AgentStartup
        Write-Step 'post-clone activation startup entry installed'
    }
    'AuditOobe' {
        Copy-AgentBinary
        Reset-AgentState
        Install-AgentStartup
        Write-Step 'Audit/OOBE image prepared; run sysprep /oobe /shutdown when ready'
    }
    'QcCleanup' {
        Copy-AgentBinary
        Remove-AgentStartup
        Reset-AgentState
        Install-AgentStartup
        Write-Step 'QC cleanup complete; state reset and startup entry installed'
    }
    'InstallOnly' {
        Copy-AgentBinary
        Install-AgentStartup
        Write-Step 'startup entry installed'
    }
    'RemoveOnly' {
        Remove-AgentStartup
        Write-Step 'startup entry removed'
    }
}

Write-Step 'done'
