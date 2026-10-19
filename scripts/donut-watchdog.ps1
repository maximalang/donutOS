# Donut-Free watchdog: health check + self-heal restart. Log: logs/watchdog.log
$ErrorActionPreference = 'SilentlyContinue'
$Base = Split-Path -Parent $MyInvocation.MyCommand.Path
$token = (Get-Content (Join-Path $Base 'config\automation-token.txt') -Raw).Trim()
$logFile = Join-Path $Base 'logs\watchdog.log'

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

if (HealthOk) { exit 0 }

Log 'WARN health failed - attempting restart'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Base 'donut-freectl.ps1') restart -Base $Base | ForEach-Object { Log "  ctl: $_" }
Start-Sleep 5
if (HealthOk) { Log 'OK restarted and healthy' } else { Log 'ERROR restart did not recover - manual attention needed' }
