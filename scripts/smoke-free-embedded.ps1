# Isolated acceptance smoke for a free DonutOS build (spec 29.10, final block).
# Runs the freshly built exe against a throwaway data root and verifies:
#   1. health + version
#   2. profile round-trip through the EMBEDDED sync server (create -> objects
#      on disk -> restart -> profile survives)
#   3. member REST: issue Operator -> call with member token -> revoke -> 401
#   4. automation-token rotation: old dies, new works
#   5. idle window: 0 WARN, 0 background network retries/fetches (signed-out)
#   6. exit leaves no webview orphans
# Usage: powershell -File smoke-free-embedded.ps1 -Exe <path-to-donutbrowser.exe> [-IdleMinutes 10]
# The automation token is minted locally and never printed.
param(
  [Parameter(Position = 0)][string]$Exe = 'D:\ZDownloads\donut-free-target\release\donutbrowser.exe',
  [string]$DataRoot = '',
  [int]$IdleMinutes = 10,
  [int]$Port = 10109
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$script:Failures = 0

if (-not $DataRoot) { $DataRoot = Join-Path $env:TEMP ("donut-smoke-" + [guid]::NewGuid().ToString('N').Substring(0, 8)) }
New-Item -ItemType Directory -Force -Path $DataRoot | Out-Null
# In portable mode the automation token lives in config\ beside the exe; an
# existing file token always wins over the env var, so smoke with whatever the
# binary will actually use. Only mint one when the exe has none yet.
$exeConfigToken = Join-Path (Split-Path $Exe -Parent) 'config\automation-token.txt'
if (Test-Path $exeConfigToken) {
  $script:token = (Get-Content $exeConfigToken -Raw).Trim()
} else {
  $script:token = [guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N')
}
$logPath = Join-Path $DataRoot 'logs\DonutBrowser.log'

# Fresh roots ship with the local API disabled (api_enabled=false by default);
# the deploy seed is what turns it on in prod. Mirror that here, pinned to the
# smoke port, so the isolated instance actually serves /v1.
$settingsDir = Join-Path $DataRoot 'data\settings'
New-Item -ItemType Directory -Force -Path $settingsDir | Out-Null
$settingsFile = Join-Path $settingsDir 'app_settings.json'
if (-not (Test-Path $settingsFile)) {
  @{ api_enabled = $true; api_port = $Port } | ConvertTo-Json | Set-Content -Path $settingsFile -Encoding ASCII
}

function Log([string]$m) { Write-Output ("{0}  {1}" -f (Get-Date -Format HH:mm:ss), $m) }
function Check([bool]$ok, [string]$what) {
  if ($ok) { Log "PASS  $what" } else { Log "FAIL  $what"; $script:Failures++ }
}
function Api([string]$Method, [string]$Path, $Body = $null, [string]$Tok = $script:token) {
  $args_ = @{ Method = $Method; Uri = "http://127.0.0.1:$Port$Path"; TimeoutSec = 10 }
  if ($Tok) { $args_.Headers = @{ Authorization = "Bearer $Tok" } }
  if ($null -ne $Body) { $args_.Body = ($Body | ConvertTo-Json); $args_.ContentType = 'application/json' }
  Invoke-RestMethod @args_
}
function Start-App() {
  $env:DONUTBROWSER_DATA_ROOT = $DataRoot
  $env:DONUT_AUTOMATION_TOKEN = $script:token
  $script:proc = Start-Process -FilePath $Exe -PassThru -WindowStyle Minimized
  $deadline = (Get-Date).AddSeconds(90)
  do {
    Start-Sleep -Milliseconds 700
    try { $null = Api GET '/v1/health'; return $script:proc } catch {}
  } while ((Get-Date) -lt $deadline)
  throw "app did not become healthy within 90s"
}
function Stop-App($proc) {
  # Ask the app to exit through its own shutdown path (window close), so the
  # webview-orphan check exercises the real exit sequence.
  $proc.CloseMainWindow() | Out-Null
  if (-not $proc.WaitForExit(20)) { Stop-Process -Id $proc.Id -Force }
  Start-Sleep -Seconds 3
}

Log "=== DonutOS free smoke: exe=$Exe data=$DataRoot idle=${IdleMinutes}m"
$webviewBefore = @(Get-Process msedgewebview2 -ErrorAction SilentlyContinue).Count

# --- 1. start + health -------------------------------------------------------
$proc = Start-App
try {
  $h = Api GET '/v1/health'
  Check ($h.status -eq 'ok' -and $h.version) "health ok, version=$($h.version) build=$($h.build)"

  # --- 2. profile round-trip through embedded sync ---------------------------
  $profName = "smoke-$([guid]::NewGuid().ToString('N').Substring(0,6))"
  $prof = Api POST '/v1/profiles' @{ name = $profName; browser = 'wayfern'; proxy_id = $null; vpn_id = $null }
  $profId = $prof.profile.id
  Check ($profId) "profile created ($profName)"
  Start-Sleep -Seconds 3
  $objects = @(Get-ChildItem (Join-Path $DataRoot 'sync-local') -Recurse -File -ErrorAction SilentlyContinue)
  Check ($objects.Count -gt 0) "embedded sync wrote objects ($($objects.Count) files)"
  Stop-App $proc
  $proc = Start-App
  $names = @((Api GET '/v1/profiles') | ForEach-Object { $_.name })
  Check ($names -contains $profName) "profile survived restart via embedded sync"

  # --- 3. member REST battery ------------------------------------------------
  $member = Api POST '/v1/members' @{ name = 'smoke-operator'; role = 'Operator' }
  Check ($member.token) "member issued (id=$($member.id) role=$($member.role))"
  $null = Api GET '/v1/profiles' $null $member.token
  Check $true "member token calls the API (Operator read)"
  $denied = $false
  try { $null = Api POST '/v1/members' @{ name = 'x'; role = 'Viewer' } $member.token; $denied = $false }
  catch { $denied = ($_.Exception.Response.StatusCode.value__ -eq 403) }
  Check $denied "member cannot reach owner-only surface (403)"
  $mid = $member.id
  $null = Api DELETE "/v1/members/$mid"
  $revoked = $false
  try { $null = Api GET '/v1/profiles' $null $member.token; $revoked = $false }
  catch { $revoked = ($_.Exception.Response.StatusCode.value__ -eq 401) }
  Check $revoked "revoked member token is dead (401)"

  # --- 4. idle window: 0 WARN, 0 network retries ------------------------------
  Log "idle ${IdleMinutes}m (signed-out, window minimized)..."
  $idleStart = (Get-Date).AddSeconds(-2)
  Start-Sleep -Seconds ($IdleMinutes * 60)
  $lines = @(Get-Content $logPath -ErrorAction SilentlyContinue | Where-Object {
    $_ -match '^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})' -and (
      [datetime]::Parse($Matches[1]) -ge $idleStart)
  })
  $warn = @($lines | Where-Object { $_ -match '\[WARN\]|\[ERROR\]' }).Count
  $net = @($lines | Where-Object { $_ -match 'Remote session stream|version fetch|Unsupported browser|connect retry' }).Count
  Check ($warn -eq 0) "idle window WARN/ERROR lines = $warn (target 0)"
  Check ($net -eq 0) "idle window network retries/fetches = $net (target 0)"

  # --- 5. token rotation (last: the old token dies here) ----------------------
  $rot = Api POST '/v1/automation-token/rotate'
  Check ($rot.token -and $rot.token -ne $script:token) "token rotated"
  $oldDead = $false
  try { $null = Api GET '/v1/health' $null $script:token; $oldDead = $false }
  catch { $oldDead = ($_.Exception.Response.StatusCode.value__ -eq 401) }
  Check $oldDead "old automation token is dead"
  $null = Api GET '/v1/health' $null $rot.token
  Check $true "new automation token works"

  # --- 6. exit leaves no webview orphans --------------------------------------
  Stop-App $proc
  Start-Sleep -Seconds 5
  $webviewAfter = @(Get-Process msedgewebview2 -ErrorAction SilentlyContinue).Count
  Check ($webviewAfter -le $webviewBefore) "no webview orphans (before=$webviewBefore after=$webviewAfter)"
} finally {
  $live = $null
  if ($script:proc -and -not $script:proc.HasExited) { $live = $script:proc }
  if ($proc -and -not $proc.HasExited) { $live = $proc }
  if ($live) { Stop-Process -Id $live.Id -Force -ErrorAction SilentlyContinue }
  $env:DONUTBROWSER_DATA_ROOT = $null
  $env:DONUT_AUTOMATION_TOKEN = $null
}

$verdict = 'FAIL'
if ($script:Failures -eq 0) { $verdict = 'PASS' }
Log ("=== SMOKE {0}: failures={1} (data root kept: {2})" -f $verdict, $script:Failures, $DataRoot)
if ($script:Failures -gt 0) { exit 1 } else { exit 0 }
