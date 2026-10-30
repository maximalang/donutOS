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
$Base = [System.IO.Path]::GetFullPath($Base)
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
Log ("health: status={0} version={1} build={2} profiles={3}" -f $h.status, $h.version, $h.build, $h.profiles_total)
$profilesBefore = $h.profiles_total
# Capture the real main PID right away: the ctl Get-Procs probe (ExecutablePath
# -like) has been observed to miss a live, healthy instance on this host.
$main = Get-CimInstance Win32_Process -Filter "Name='DonutOS.exe'" | Select-Object -First 1
if ($main) { Log ("main pid: {0}" -f $main.ProcessId) } else { Log "WARN: no DonutOS.exe process found while health answers" }

# 2. Idle window 1
Log "idle ${Idle1Sec}s (window open, signed-out)"
Start-Sleep -Seconds $Idle1Sec

# 3. Create test profile (response shape: { profile: { id, name, browser, ... } })
$profName = "perf-$Tag-" + (Get-Date -Format 'HHmmss')
try {
  $prof = Api POST '/v1/profiles' @{ name = $profName; browser = 'wayfern'; proxy_id = $null; vpn_id = $null }
  $profId = $prof.profile.id
  Log ("created profile: id={0} name={1} engine={2}" -f $profId, $prof.profile.name, $prof.profile.browser)
} catch { Log "FATAL: profile create failed: $($_.Exception.Message)"; & powershell.exe -NoProfile -File $ctl stop -Base $Base | Out-Null; exit 1 }
if (-not $profId) { Log "FATAL: profile create returned no id"; & powershell.exe -NoProfile -File $ctl stop -Base $Base | Out-Null; exit 1 }

# 4. Open/close x3 (visible window, about:blank)
for ($i = 1; $i -le 3; $i++) {
  try {
    $r = Api POST "/v1/profiles/$profId/run" @{ url = 'about:blank' }
    Log ("run #{0} ok: attached={1} cdp_port={2}" -f $i, $r.attached, $r.remote_debugging_port)
  } catch { Log ("run #$i FAILED: " + $_.Exception.Message); break }
  Start-Sleep -Seconds 30
  try { Api POST "/v1/profiles/$profId/kill" | Out-Null; Log "kill #$i ok" }
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
try { Api DELETE "/v1/profiles/$profId" | Out-Null; Log "test profile removed" }
catch { Log ("profile cleanup failed (manual cleanup needed): " + $_.Exception.Message) }
$h2 = Api GET '/v1/health'
Log ("health after cleanup: profiles=$($h2.profiles_total) (before=$profilesBefore)")

# 8. Stop via ctl, then verify the instance is really gone (the ctl probe can
# miss a live process); report leftovers so the operator can finish by PID tree.
$stopOut = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ctl stop -Base $Base
Log ("ctl stop: " + ($stopOut -join ' | '))
Start-Sleep -Seconds 3
$left = Get-CimInstance Win32_Process -Filter "Name='DonutOS.exe'"
if ($left) { Log ("WARN: DonutOS.exe still alive after ctl stop: " + (($left | ForEach-Object { $_.ProcessId }) -join ',')) }
else { Log "verified: no DonutOS.exe process left" }
Log "=== scenario $Tag end ==="
