# Donut-Free watchdog: health check + self-heal restart. Log: logs/watchdog.log
#
# Two operating models (the choice belongs to the operator/owner):
#   on-demand (default) - restart ONLY after a crash: the process was healthy
#     at the previous tick (state file), went down WITHOUT `ctl stop` (no
#     marker file), and had been up less than -MaxAgeHours. A clean
#     `donut-freectl stop` and a long-lived instance going down are both
#     treated as intentional: no restart.
#   -AlwaysOn - restart on any downtime (kiosk/unattended). This is the
#     previous behaviour, including resurrection after an intentional stop.
param(
  [switch]$AlwaysOn,
  [int]$MaxAgeHours = 8
)
$ErrorActionPreference = 'SilentlyContinue'
$Base = Split-Path -Parent $MyInvocation.MyCommand.Path
$token = (Get-Content (Join-Path $Base 'config\automation-token.txt') -Raw).Trim()
$logDir   = Join-Path $Base 'logs'
$logFile  = Join-Path $logDir 'watchdog.log'
$stateF   = Join-Path $logDir 'watchdog-state.json'
$stopFlag = Join-Path $logDir 'ctl-stop.flag'

function Log($msg) {
  $line = "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $msg"
  Add-Content -Path $logFile -Value $line
  if ((Get-Item $logFile -ErrorAction SilentlyContinue).Length -gt 1MB) {
    Move-Item $logFile "$logFile.old" -Force
  }
}

function HealthOk {
  try {
    $r = Invoke-WebRequest "http://127.0.0.1:10109/v1/health" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 8 -UseBasicParsing
    return ($r.StatusCode -eq 200)
  } catch { return $false }
}

function Save-State {
  # Remember when the healthy instance started, so a later dead tick can tell
  # a young crash from an end-of-life stop. First creation date of any process
  # whose exe lives under $Base (same identity rule as donut-freectl).
  $procs = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($Base) })
  if (-not $procs.Count) { return }
  $started = @($procs | ForEach-Object { $_.CreationDate } | Where-Object { $_ } | Sort-Object | Select-Object -First 1)
  if (-not $started) { return }
  @{ ts = (Get-Date).ToString('s'); started = $started[0].ToString('s') } |
    ConvertTo-Json -Compress | Set-Content -Path $stateF -Encoding ASCII
}

function Read-State {
  if (Test-Path $stateF) {
    try { return (Get-Content $stateF -Raw | ConvertFrom-Json) } catch { return $null }
  }
  return $null
}

if (HealthOk) { Save-State; exit 0 }

$mode = if ($AlwaysOn) { 'always-on' } else { 'on-demand' }

if ($AlwaysOn) {
  Log "WARN health failed - attempting restart (mode=$mode)"
} else {
  # 1. A clean `ctl stop` leaves a marker (donut-freectl.ps1 writes it before
  #    killing, its start clears it). Restarting here would fight the operator.
  if (Test-Path $stopFlag) {
    Log "INFO health down but ctl-stop marker present - intentional stop, not restarting (mode=$mode)"; exit 0
  }
  # 2. No crash evidence: this watchdog never saw the instance healthy, so a
  #    start/stop that happened entirely between ticks stays stopped.
  $st = Read-State
  if (-not $st -or -not $st.ts -or -not $st.started) {
    Log "INFO health down, no prior healthy state - not restarting (mode=$mode)"; exit 0
  }
  # 3. A long-lived instance going down reads as an intentional stop, not a
  #    young crash: uptime is measured at the last healthy tick.
  $upHours = ((Get-Date $st.ts) - (Get-Date $st.started)).TotalHours
  if ($upHours -ge $MaxAgeHours) {
    Log ("INFO health down after {0:n1}h uptime (>= {1}h) - treating as intentional stop, not restarting (mode={2})" -f $upHours, $MaxAgeHours, $mode)
    exit 0
  }
  Log ("WARN crash detected: healthy at {0}, up {1:n1}h (< {2}h) - attempting restart (mode={3})" -f $st.ts, $upHours, $MaxAgeHours, $mode)
}

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Base 'donut-freectl.ps1') restart -Base $Base | ForEach-Object { Log "  ctl: $_" }
Start-Sleep 5
if (HealthOk) { Log 'OK restarted and healthy'; Save-State } else { Log 'ERROR restart did not recover - manual attention needed' }
