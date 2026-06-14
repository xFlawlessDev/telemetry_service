<#
.SYNOPSIS
Deploy Telemetry Service activation agent for manufacturing Windows images.

.DESCRIPTION
Copies telemetry_service.exe to Program Files, resets activation state/logs, and manages
TelemetryServiceActivation Scheduled Task through the agent CLI.

Modes:
- UserModeMaster: prepare a clone master; copy binary, remove task, reset state.
- PostClone: run on final cloned machine; copy binary, reset state, install task.
- AuditOobe: run in Audit Mode before sysprep /oobe /shutdown; copy binary, reset state, install task.
- QcCleanup: run after QC test; remove task, reset state, install task.
- InstallOnly: copy binary and install task.
- RemoveOnly: remove task only.

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

function Install-AgentTask {
    Invoke-Agent -Arguments @('--install-task')
}

function Remove-AgentTask {
    Invoke-Agent -Arguments @('--remove-task')
}

if (-not $SkipAdminCheck -and -not (Test-IsAdministrator)) {
    throw 'Run this script from an elevated PowerShell session, or pass -SkipAdminCheck if your environment grants equivalent rights.'
}

Write-Step "mode: $Mode"

switch ($Mode) {
    'UserModeMaster' {
        Copy-AgentBinary
        Remove-AgentTask
        Reset-AgentState
        Write-Step 'master prepared; do not install active task until post-clone'
    }
    'PostClone' {
        Copy-AgentBinary
        Reset-AgentState
        Install-AgentTask
        Write-Step 'post-clone activation task installed'
    }
    'AuditOobe' {
        Copy-AgentBinary
        Reset-AgentState
        Install-AgentTask
        Write-Step 'Audit/OOBE image prepared; run sysprep /oobe /shutdown when ready'
    }
    'QcCleanup' {
        Copy-AgentBinary
        Remove-AgentTask
        Reset-AgentState
        Install-AgentTask
        Write-Step 'QC cleanup complete; state reset and task installed'
    }
    'InstallOnly' {
        Copy-AgentBinary
        Install-AgentTask
        Write-Step 'task installed'
    }
    'RemoveOnly' {
        Remove-AgentTask
        Write-Step 'task removed'
    }
}

Write-Step 'done'
