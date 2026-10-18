# Donut-Free control: status | start | stop | restart | token | doctor | backup | restore
param(
  [Parameter(Position = 0)][string]$Action = "status",
  [string]$Base,
  [string]$BackupDir
)
$ErrorActionPreference = 'SilentlyContinue'
if (-not $Base) { $Base = Split-Path -Parent $MyInvocation.MyCommand.Path }
$exe       = Join-Path $Base 'DonutFree.exe'
$tokenFile = Join-Path $Base 'config\automation-token.txt'
$settingsF = Join-Path $Base 'data\settings\app_settings.json'
$token   = if (Test-Path $tokenFile) { (Get-Content $tokenFile -Raw).Trim() } else { $null }
$port    = $null
if (Test-Path $settingsF) { $port = (Get-Content $settingsF -Raw | ConvertFrom-Json).api_port }
if (-not $port) { $port = 10109 }

function Get-Procs {
  Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$Base*" } |
    ForEach-Object { Get-Process -Id $_.ProcessId }
}
function Api([string]$Path) {
  if (-not $token) { return $null }
  Invoke-RestMethod -Uri "http://127.0.0.1:$port$Path" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 5
}
function Wait-Port([int]$Seconds) {
  $deadline = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $deadline) {
    if (Api '/v1/health') { return $true }
    Start-Sleep -Milliseconds 700
  }
  return $false
}

switch ($Action) {
  'status' {
    $p = @(Get-Procs)
    "processes : $($p.Count)$(if ($p.Count) { ' (pid ' + ($p.Id -join ',') + ')' })"
    $h = Api '/v1/health'
    if ($h) { "api       : http://127.0.0.1:$port OK  v$($h.version) build=$($h.build) profiles=$($h.profiles_total)" }
    else    { "api       : http://127.0.0.1:$port NOT responding" }
  }
  'start' {
    if (@(Get-Procs).Count) { "already running"; break }
    if (-not (Test-Path $exe)) { "ERROR: $exe not found"; break }
    $env:DONUT_AUTOMATION_TOKEN = $token
    $proc = Start-Process -FilePath $exe -WorkingDirectory $Base -PassThru
    if (Wait-Port 60) {
      # Guard against the single-instance race: a just-killed sibling may
      # still answer health while the spawned child self-exits silently.
      Start-Sleep 2
      $proc.Refresh()
      $alive = ($proc -and -not $proc.HasExited)
      $n = @(Get-Procs).Count
      if ($alive -and $n -eq 1) { $h = Api '/v1/health'; "STARTED  http://127.0.0.1:$port  v$($h.version) build=$($h.build) (pid $($proc.Id))" }
      else { "FAILED: spawned child not healthy (alive=$alive procs=$n) - single-instance race or early exit"; break }
    }
    else { "WARN: process started but /v1/health not ready in 60s (first-run engine download can take minutes)" }
  }
  'stop' {
    $p = @(Get-Procs)
    if (-not $p.Count) { "not running"; break }
    $p | Stop-Process -Force
    # Wait until the processes are really gone AND the port is released,
    # so a dying instance cannot answer the next start's health probe.
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and @(Get-Procs).Count) { Start-Sleep -Milliseconds 300 }
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Date) -lt $deadline) {
      $conn = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
      if (-not $conn) { break }
      Start-Sleep -Milliseconds 300
    }
    $left = @(Get-Procs).Count
    $warn = if ($left) { " WARN: $left still alive" } else { '' }
    "STOPPED $($p.Count) process(es)$warn"
  }
  'restart' {
    # Re-invoking $PSCommandPath in-process proved silent under -File on this host;
    # use an explicit child powershell so stop/start run their full logic.
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSCommandPath" stop -Base $Base
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSCommandPath" start -Base $Base
  }
  'token' {
    if ($token) { "port: $port`ntoken: $token`nheader: Authorization: Bearer $token" }
    else { "no token file at $tokenFile" }
  }
  'doctor' {
    "exe       : $(if (Test-Path $exe) { 'OK ' + $exe } else { 'MISSING ' + $exe })"
    "portable  : $(if (Test-Path (Join-Path $Base '.portable')) { 'OK (.portable present)' } else { 'MISSING .portable' })"
    "settings  : $(if (Test-Path $settingsF) { 'OK port=' + $port } else { 'MISSING (defaults: api off!)' })"
    "token     : $(if ($token) { 'OK len=' + $token.Length } else { 'MISSING' })"
    "proxy     : $(if (Test-Path (Join-Path $Base 'donut-proxy.exe')) { 'OK' } else { 'missing' })"
    $conn = Get-NetTCPConnection -LocalPort $port -State Listen
    "port      : $(if ($conn) { 'LISTENING (pid ' + (($conn.OwningProcess | Select-Object -Unique) -join ',') + ')' } else { 'free' })"
    $h = Api '/v1/health'
    "health    : $(if ($h) { 'OK v' + $h.version + ' build=' + $h.build } else { 'FAIL' })"
    $d = Get-PSDrive C
    "disk C    : $([math]::Round($d.Free/1GB,1)) GB free"
  }
  'backup' {
    if (-not $BackupDir) { $BackupDir = Join-Path $Base ('backups' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
    New-Item -ItemType Directory -Force -Path $BackupDir | Out-Null
    if (Test-Path "$Base\data") { Copy-Item -Recurse -Force "$Base\data" "$BackupDir\data" }
    if (Test-Path $tokenFile)   { Copy-Item -Force $tokenFile "$BackupDir" }
    "BACKUP_OK -> $BackupDir"
  }
  'restore' {
    if (-not $BackupDir) { "usage: restore -BackupDir <path>"; break }
    if (-not (Test-Path "$BackupDir\data")) { "ERROR: no data in $BackupDir"; break }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSCommandPath" stop -Base $Base
    Copy-Item -Recurse -Force "$BackupDir\data" "$Base\data"
    if (Test-Path "$BackupDir\automation-token.txt") { New-Item -ItemType Directory -Force -Path (Split-Path $tokenFile) | Out-Null; Copy-Item -Force "$BackupDir\automation-token.txt" $tokenFile }
    "RESTORE_OK from $BackupDir"
  }
  default { "unknown action: $Action (status|start|stop|restart|token|doctor|backup|restore)" }
}
