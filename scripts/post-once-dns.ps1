<#
.SYNOPSIS
    Runs one real telemetry activation attempt with a temporary DNS override.

.DESCRIPTION
    Some networks resolve the API host (for example register.axiooworld.com) to a
    local mirror that serves an expired certificate, so activation always fails
    there. This script maps the API host to the public address of the real server
    through the hosts file for the duration of a single activation attempt.

    Safety rules:
      * requires elevation; when not elevated it re-launches itself through UAC
      * verifies the override target serves a valid TLS certificate BEFORE the
        hosts file is touched
      * refuses to run when the hosts file already maps the API host
      * writes a timestamped backup of the hosts file to %TEMP%
      * always removes the entry again in a finally block, even on failure

    Invoked by scripts\post-once.cmd when --dns is passed. It can also be run
    directly, see the usage message.

.PARAMETER AgentArgs
    Arguments forwarded from post-once.cmd:
      [path\to\telemetry_service.exe] [nopause] [--dns[=<ip>]] [--dns-child]

.EXAMPLE
    .\scripts\post-once.cmd --dns
    .\scripts\post-once.cmd --dns=104.26.12.124
    .\scripts\post-once.cmd .\target\debug\telemetry_service.exe --dns nopause
#>
[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $AgentArgs = @()
)

$ErrorActionPreference = 'Stop'

$DefaultHost    = 'register.axiooworld.com'
$Marker         = '# telemetry-post-once-dns'
$TimeoutSeconds = 240

$HostsPath = Join-Path $env:SystemRoot 'System32\drivers\etc\hosts'
$LogPath   = Join-Path $env:TEMP 'telemetry_post_once_dns.log'

function Write-Log {
    param([string] $Text = '')
    Write-Host $Text
    Add-Content -LiteralPath $LogPath -Value $Text -Encoding UTF8
}

function Quote-Arg {
    param([string] $Value)
    if ($Value -match '\s') { return '"' + $Value + '"' }
    return $Value
}

# ---------------------------------------------------------------- arguments --
$TargetExeArg = ''
$NoPause      = $false
$IsChild      = $false
$DnsRequested = $false
$DnsIp        = ''

for ($i = 0; $i -lt $AgentArgs.Count; $i++) {
    $arg = $AgentArgs[$i]
    if ([string]::IsNullOrWhiteSpace($arg)) { continue }

    if ($arg -ieq 'nopause' -or $arg -ieq 'nopause-child') { $NoPause = $true; continue }
    if ($arg -ieq '--dns-child')  { $IsChild = $true; continue }
    if ($arg -ieq 'target-exe') {
        if ($i + 1 -lt $AgentArgs.Count) { $i++; $TargetExeArg = $AgentArgs[$i] }
        continue
    }
    if ($arg -imatch '^--dns(?:=(?<ip>.+))?$') {
        $DnsRequested = $true
        if ($Matches['ip']) { $DnsIp = $Matches['ip'].Trim() }
        continue
    }
    if ([string]::IsNullOrWhiteSpace($TargetExeArg)) { $TargetExeArg = $arg }
}

if (-not $DnsRequested) {
    Write-Host 'usage: post-once-dns.ps1 [path\to\telemetry_service.exe] [nopause] --dns[=<ip>]'
    exit 1
}

# The elevated child starts in a different working directory, so a relative
# agent path has to be turned into an absolute one before the re-launch.
$AbsoluteTargetArg = ''
if (-not [string]::IsNullOrWhiteSpace($TargetExeArg)) {
    $resolved = Resolve-Path -LiteralPath $TargetExeArg -ErrorAction SilentlyContinue
    if ($resolved) {
        $AbsoluteTargetArg = $resolved.Path
    } else {
        try { $AbsoluteTargetArg = [System.IO.Path]::GetFullPath($TargetExeArg) } catch { $AbsoluteTargetArg = $TargetExeArg }
    }
}

function Resolve-TargetExe {
    param([string] $Requested)
    if (-not [string]::IsNullOrWhiteSpace($Requested)) { return $Requested }
    $installed = 'C:\Program Files\TelemetryService\telemetry_service.exe'
    if (Test-Path -LiteralPath $installed) { return $installed }
    return (Join-Path $PSScriptRoot 'telemetry_service.exe')
}

# Reads TELEMETRY_BASE_URL from the repo .env so the override targets the host
# this build actually posts to. Falls back to the documented default.
function Get-ApiTarget {
    $baseUrl = "https://$DefaultHost"
    $envFile = Join-Path $PSScriptRoot '..\.env'
    if (Test-Path -LiteralPath $envFile) {
        foreach ($line in (Get-Content -LiteralPath $envFile)) {
            $trimmed = $line.Trim()
            if (-not $trimmed -or $trimmed.StartsWith('#') -or -not $trimmed.StartsWith('TELEMETRY_BASE_URL=')) { continue }
            $value = $trimmed.Substring('TELEMETRY_BASE_URL='.Length).Trim().Trim('"').Trim("'")
            if ($value) { $baseUrl = $value.TrimEnd('/') }
            break
        }
    }
    try   { $hostName = ([uri]$baseUrl).Host }
    catch { $hostName = $DefaultHost; $baseUrl = "https://$DefaultHost" }
    return [pscustomobject]@{ Host = $hostName; BaseUrl = $baseUrl }
}

$isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)

if ($IsChild -and -not $isAdmin) {
    Write-Host '[post-once-dns] internal error: elevated child is not elevated'
    exit 1
}

# ------------------------------------------------------------- not elevated --
# Re-launch through UAC, wait for it, then surface the child's log in this
# console so results are readable without chasing the elevated window.
if (-not $isAdmin) {
    $hostExe = (Get-Process -Id $PID).Path

    $relaunch = New-Object System.Collections.Generic.List[string]
    foreach ($item in @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath)) { $relaunch.Add($item) }
    if ($AbsoluteTargetArg) { $relaunch.Add('target-exe'); $relaunch.Add($AbsoluteTargetArg) }
    $relaunch.Add('nopause-child')
    if ($DnsIp) { $relaunch.Add("--dns=$DnsIp") } else { $relaunch.Add('--dns') }
    $relaunch.Add('--dns-child')

    $argLine = ($relaunch | ForEach-Object { Quote-Arg $_ }) -join ' '

    Write-Host '[post-once-dns] requesting elevation (UAC prompt)...'
    try {
        $child = Start-Process -FilePath $hostExe -Verb RunAs -ArgumentList $argLine -PassThru
    } catch {
        Write-Host '[post-once-dns] elevation was cancelled or denied; nothing was changed'
        exit 1
    }

    $child.WaitForExit()
    if (Test-Path -LiteralPath $LogPath) {
        Write-Host ''
        Get-Content -LiteralPath $LogPath | ForEach-Object { Write-Host $_ }
    }
    if (-not $NoPause) { [void](Read-Host 'press Enter to close') }
    exit $child.ExitCode
}

# ------------------------------------------------------- elevated work start --
Set-Content -LiteralPath $LogPath -Value $null -Encoding UTF8

$targetExe = Resolve-TargetExe -Requested $AbsoluteTargetArg
if (-not (Test-Path -LiteralPath $targetExe)) {
    Write-Log "[post-once-dns] agent executable not found: $targetExe"
    if (-not $NoPause -and -not $IsChild) { [void](Read-Host 'press Enter to close') }
    exit 1
}

$api = Get-ApiTarget
$hostName = $api.Host
$baseUrl  = $api.BaseUrl

Write-Log '=== post-once with temporary DNS override ==='
Write-Log "started   : $(Get-Date -Format o)"
Write-Log "agent exe : $targetExe"
Write-Log "api host  : $hostName"
Write-Log ''

# Resolve the real address through public DNS unless the caller passed one.
if (-not $DnsIp) {
    foreach ($resolver in @('1.1.1.1', '8.8.8.8', '9.9.9.9')) {
        try {
            $resolved = Resolve-DnsName -Name $hostName -Type A -Server $resolver -DnsOnly -QuickTimeout -ErrorAction Stop |
                Where-Object { $_.IPAddress } | Select-Object -ExpandProperty IPAddress -Unique
            if ($resolved) { $DnsIp = $resolved[0]; break }
        } catch { }
    }
}
if (-not $DnsIp) {
    Write-Log '[post-once-dns] could not resolve the API host through public DNS; pass --dns=<ip>'
    if (-not $NoPause -and -not $IsChild) { [void](Read-Host 'press Enter to close') }
    exit 1
}
Write-Log "override  : $hostName -> $DnsIp"

# Verify the target actually serves a valid certificate before changing hosts.
# This is what keeps the script from breaking name resolution for a host that
# has no working endpoint.
$curl = Get-Command curl.exe -ErrorAction SilentlyContinue
if ($curl) {
    # The point of the probe is that TLS validates for this host on this
    # address, so ssl_verify_result must be 0. The HTTP status is only used to
    # prove the request completed (a redirect is fine); the base URL can answer
    # 301 without a trailing slash.
    $probe = curl.exe -sS --resolve "${hostName}:443:${DnsIp}" -o NUL -w '%{http_code} %{ssl_verify_result}' --max-time 20 "$baseUrl/" 2>&1
    $probeText = ($probe | Out-String).Trim()
    Write-Log "probe     : $probeText"

    $probeOk = $false
    if ($probeText -match '^(?<code>\d{3}) (?<verify>\d+)$') {
        $probeCode   = $Matches['code']
        $probeVerify = $Matches['verify']
        $probeOk = ($probeVerify -eq '0') -and ($probeCode -ne '000')
    }

    if (-not $probeOk) {
        Write-Log "[post-once-dns] $DnsIp does not serve a trusted certificate for $hostName"
        Write-Log '[post-once-dns] hosts file left untouched'
        if (-not $NoPause -and -not $IsChild) { [void](Read-Host 'press Enter to close') }
        exit 1
    }
} else {
    Write-Log '[post-once-dns] curl.exe not found; skipping the pre-flight certificate probe'
}

# Never fight an intentional mapping that is already in place.
$original = @(Get-Content -LiteralPath $HostsPath)
$alreadyMapped = $original |
    Where-Object { -not $_.TrimStart().StartsWith('#') -and -not $_.Contains($Marker) } |
    ForEach-Object { ($_ -split '\s+') | Select-Object -Skip 1 } |
    Where-Object { $_ -eq $hostName }
if ($alreadyMapped) {
    Write-Log "[post-once-dns] hosts already maps $hostName; refusing to change it"
    if (-not $NoPause -and -not $IsChild) { [void](Read-Host 'press Enter to close') }
    exit 1
}

$backupPath = Join-Path $env:TEMP ("telemetry_hosts_backup_{0}.txt" -f (Get-Date -Format 'yyyyMMdd_HHmmss'))
Set-Content -LiteralPath $backupPath -Value $original -Encoding ASCII
Write-Log "backup    : $backupPath"
Write-Log ''

$agentOut  = Join-Path $env:TEMP 'telemetry_post_once_dns_stdout.txt'
$agentErr  = Join-Path $env:TEMP 'telemetry_post_once_dns_stderr.txt'
$agentExit = 1
$restored  = $false

try {
    Add-Content -LiteralPath $HostsPath -Value "$DnsIp $hostName $Marker" -Encoding ASCII
    ipconfig /flushdns | Out-Null
    Write-Log "hosts     : added temporary entry for $hostName"
    Write-Log ''
    Write-Log '=== agent: --once --print-payload ==='

    $startedAt = Get-Date
    $proc = Start-Process -FilePath $targetExe -ArgumentList '--once', '--print-payload' -NoNewWindow -PassThru `
        -RedirectStandardOutput $agentOut -RedirectStandardError $agentErr
    $finished = $proc.WaitForExit($TimeoutSeconds * 1000)
    if (-not $finished) {
        $proc.Kill()
        Write-Log "agent exceeded ${TimeoutSeconds}s and was killed"
        $agentExit = 1
    } else {
        # Refresh first: with redirected output the ExitCode can still read as
        # null right after a timed WaitForExit.
        $proc.Refresh()
        $code = $proc.ExitCode
        if ($null -eq $code) { $code = 0 }
        $agentExit = $code
        Write-Log "elapsed   : $([math]::Round(((Get-Date) - $startedAt).TotalSeconds, 1))s"
        Write-Log "exit code : $agentExit"
    }

    if (Test-Path -LiteralPath $agentOut) {
        $stdout = (Get-Content -LiteralPath $agentOut -Raw)
        if ($stdout -and $stdout.Trim()) { Write-Log "stdout    : $($stdout.Trim())" }
    }
    if (Test-Path -LiteralPath $agentErr) {
        $stderr = (Get-Content -LiteralPath $agentErr -Raw)
        if ($stderr -and $stderr.Trim()) { Write-Log "stderr    : $($stderr.Trim())" }
    }
} catch {
    Write-Log "[post-once-dns] ERROR: $($_.Exception.Message)"
    $agentExit = 1
} finally {
    # Restore the hosts file no matter how the attempt ended.
    $cleaned = @(Get-Content -LiteralPath $HostsPath | Where-Object { -not $_.Contains($Marker) })
    Set-Content -LiteralPath $HostsPath -Value $cleaned -Encoding ASCII
    ipconfig /flushdns | Out-Null
    $restored = $true

    $leftover = @(Select-String -LiteralPath $HostsPath -Pattern $Marker -SimpleMatch -ErrorAction SilentlyContinue).Count
    Write-Log ''
    if ($leftover -eq 0) {
        Write-Log 'hosts     : temporary entry removed'
    } else {
        Write-Log "[post-once-dns] WARNING: $leftover marker line(s) remain in $HostsPath"
        Write-Log "[post-once-dns] restore manually from $backupPath"
    }
}

Write-Log ''
Write-Log 'exit code 0 from --once does not prove activation succeeded; it is also'
Write-Log 'what an unreachable server returns. Confirm the new record through the'
Write-Log 'get_device_on endpoint.'

if (-not $NoPause -and -not $IsChild) { [void](Read-Host 'press Enter to close') }
if (-not $restored) { exit 1 }
exit $agentExit
