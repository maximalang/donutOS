# DonutOS measurement pass driver (spec Etap 0 / final control pass).
# Scenario: ctl start -> health -> 10 min idle -> create test profile ->
# open/close x3 -> 5 min idle -> snapshot t1 -> 10 min idle -> snapshot t2 ->
# delete test profile -> ctl stop.
# The automation token is read from the install dir and never printed.
# Usage: powershell -File perf-scenario.ps1 -Base <install-dir> -OutDir <dir> [-Tag baseline]
param(
  [string]$Base = 'C:\Users\max\Desktop\DonutOS',
  [string]$OutDir = '',
  [string]$Tag = 'baseline',
  [int]$Idle1Sec = 600,
  [int]$Idle2Sec = 300,
  [int]$Idle3Sec = 600
)
$ErrorActionPreference = 'Continue'
$repoScripts = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $OutDir) { $OutDir = Join-Path $repoScripts "..\perf-runs\$Tag" }
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$progressLog = Join-Path $OutDir 'scenario.log'
$ctl = Join-Path $Base 'donut-freectl.ps1'
$tokenFile = Join-Path $Base 'config\automation-token.txt'
$settingsF = Join-Path $Base 'data\settings\app_settings.json'
$port = 10109
if (Test-Path $settingsF) { $p = (Get-Content $settingsF -Raw | ConvertFrom-Json).api_port; if ($p) { $port = $p } }

function Log([string]$m) {
  $line = "{0} {1}" -f (Get-Date -Format 'HH:mm:ss'), $m
  Add-Content -Path $progressLog -Value $line -Encoding UTF8
  Write-Output $line
}
function Api([string]$Method, [string]$Path, $Body = $null) {
  $token = (Get-Content $tokenFile -Raw).Trim()
  $args_ = @{
    Method      = $Method
    Uri         = "http://127.0.0.1:$port$Path"
    Headers     = @{ Authorization = "Bearer $token" }
    TimeoutSec  = 30
    ContentType = 'application/json'
  }
  if ($null -ne $Body) { $args_.Body = ($Body | ConvertTo-Json -Depth 5) }
  Invoke-RestMethod @args_
}
function Snap([string]$label) {
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoScripts 'perf-snapshot.ps1') $label -Base $Base -OutDir $OutDir
}

Log "=== scenario $Tag start (pid $PID) ==="
# 1. Start via ctl and confirm health
$startOut = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ctl start -Base $Base
Log ("ctl start: " + ($startOut -join ' | '))
$h = Api GET '/v1/health'
if (-not $h) { Log "FATAL: health not responding"; exit 1 }
Log ("health: version=$($h.version) build=$($h.build) profiles=$($h.profiles_total) mode=$($h.mode)")
$profilesBefore = $h.profiles_total

# 2. Idle window 1
Log "idle ${Idle1Sec}s (window open, signed-out)"
Start-Sleep -Seconds $Idle1Sec

# 3. Create test profile
$profName = "perf-$Tag-" + (Get-Date -Format 'HHmmss')
try {
  $prof = Api POST '/v1/profiles' @{ name = $profName; browser = 'wayfern'; proxy_id = $null; vpn_id = $null }
  Log ("created profile: id={0} name={1} engine={2}" -f $prof.id, $prof.name, $prof.browser)
} catch { Log "FATAL: profile create failed: $($_.Exception.Message)"; & powershell.exe -NoProfile -File $ctl stop -Base $Base | Out-Null; exit 1 }

# 4. Open/close x3 (visible window, about:blank)
for ($i = 1; $i -le 3; $i++) {
  try {
    $r = Api POST "/v1/profiles/$($prof.id)/run" @{ url = 'about:blank' }
    Log ("run #$i ok: browser_pid={0}" -f $r.pid)
  } catch { Log ("run #$i FAILED: " + $_.Exception.Message); break }
  Start-Sleep -Seconds 30
  try { Api POST "/v1/profiles/$($prof.id)/kill" | Out-Null; Log "kill #$i ok" }
  catch { Log ("kill #$i failed: " + $_.Exception.Message) }
  Start-Sleep -Seconds 15
}

# 5. Idle window 2 then snapshot t1
Log "idle ${Idle2Sec}s after open/close"
Start-Sleep -Seconds $Idle2Sec
Snap "t1-$Tag" | Out-Null
Log "snapshot t1 done"

# 6. Plateau window then snapshot t2
Log "plateau idle ${Idle3Sec}s"
Start-Sleep -Seconds $Idle3Sec
Snap "t2-$Tag" | Out-Null
Log "snapshot t2 done"

# 7. Remove test profile
try { Api DELETE "/v1/profiles/$($prof.id)" | Out-Null; Log "test profile removed" }
catch { Log ("profile cleanup failed (manual cleanup needed): " + $_.Exception.Message) }
$h2 = Api GET '/v1/health'
Log ("health after cleanup: profiles=$($h2.profiles_total) (before=$profilesBefore)")

# 8. Stop via ctl
$stopOut = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ctl stop -Base $Base
Log ("ctl stop: " + ($stopOut -join ' | '))
Log "=== scenario $Tag end ==="
