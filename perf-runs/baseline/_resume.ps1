# One-off resume of measurement pass #1 after run 14 died mid-scenario.
# Prod (pid 628) never stopped: profile created, 3x open/close done at 12:24:03,
# idle ever since. This resumes at step 5: t1 snapshot now, 10 min plateau,
# t2 snapshot, test profile cleanup, ctl stop, leftover check.
$ErrorActionPreference = 'Continue'
$Base = 'C:\Users\max\Desktop\DonutOS'
$OutDir = 'C:\Users\max\donut-free\perf-runs\baseline'
$repoScripts = 'C:\Users\max\donut-free\scripts'
$progressLog = Join-Path $OutDir 'scenario.log'
$ctl = Join-Path $Base 'donut-freectl.ps1'
$tokenFile = Join-Path $Base 'config\automation-token.txt'
$port = 10109
$profId = '74f5a149-7049-4377-9d3e-c690d14d029a'

function Log([string]$m) {
  $line = "{0} {1}" -f (Get-Date -Format 'HH:mm:ss'), $m
  Add-Content -Path $progressLog -Value $line -Encoding UTF8
  Write-Output $line
}
function Api([string]$Method, [string]$Path, $Body = $null) {
  $token = (Get-Content $tokenFile -Raw).Trim()
  $a = @{ Method = $Method; Uri = "http://127.0.0.1:$port$Path"; Headers = @{ Authorization = "Bearer $token" }; TimeoutSec = 30; ContentType = 'application/json' }
  if ($null -ne $Body) { $a.Body = ($Body | ConvertTo-Json -Depth 5) }
  Invoke-RestMethod @a
}
function Snap([string]$label) {
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repoScripts 'perf-snapshot.ps1') $label -Base $Base -OutDir $OutDir
}

Log "=== scenario baseline RESUME (pid $PID, prod untouched since 12:11) ==="
# Window state probe: minimized windows get trimmed RSS and skew the plateau.
Add-Type -Namespace Win32 -Name Native -MemberDefinition '[DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);'
$gp = Get-Process -Name DonutOS -ErrorAction SilentlyContinue | Select-Object -First 1
if ($gp) { Log ("main window: hwnd={0} minimized={1} title='{2}'" -f $gp.MainWindowHandle, [Win32.Native]::IsIconic($gp.MainWindowHandle), $gp.MainWindowTitle) }
else { Log "FATAL: DonutOS.exe not running"; exit 1 }

# t1 (idle since 12:24:03, ~46 min -- longer than spec 5 min, same plateau)
Snap 't1-baseline' | Out-Null
Log "snapshot t1 done (resumed; idle since 12:24:03)"

# Plateau window then t2
Log "plateau idle 600s"
Start-Sleep -Seconds 600
Snap 't2-baseline' | Out-Null
Log "snapshot t2 done"

# Test profile cleanup
try { Api DELETE "/v1/profiles/$profId" | Out-Null; Log "test profile removed" }
catch { Log ("profile cleanup failed: " + $_.Exception.Message) }
$h2 = Api GET '/v1/health'
Log ("health after cleanup: profiles=$($h2.profiles_total)")

# Capture webview children BEFORE stop (for O6 comparison after stop)
$wvBefore = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.ParentProcessId -eq 628 -or ($_.CommandLine -like "*DonutOS*") })
Log ("webview processes before stop: {0} ({1})" -f $wvBefore.Count, (($wvBefore | ForEach-Object { $_.ProcessId }) -join ','))

# Stop via ctl
$stopOut = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ctl stop -Base $Base
Log ("ctl stop: " + ($stopOut -join ' | '))
Start-Sleep -Seconds 5
$left = Get-CimInstance Win32_Process -Filter "Name='DonutOS.exe'"
if ($left) { Log ("WARN: DonutOS.exe still alive after ctl stop: " + (($left | ForEach-Object { $_.ProcessId }) -join ',')) }
else { Log "verified: no DonutOS.exe process left" }
# Orphan webview check (O6): any msedgewebview2 whose parent is gone and whose
# command line references the DonutOS install dir (never kill by name alone).
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine
$alive = @{}; foreach ($p in $all) { $alive[[int]$p.ProcessId] = $true }
$orphans = @($all | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and -not $alive.ContainsKey([int]$_.ParentProcessId) -and $_.CommandLine -like "*$Base*" })
Log ("orphan webview after stop: {0} ({1})" -f $orphans.Count, (($orphans | ForEach-Object { $_.ProcessId }) -join ','))
Log "=== scenario baseline end ==="
