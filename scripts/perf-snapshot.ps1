# DonutOS perf snapshot: one JSON measurement point for a running instance.
# Captures the full process tree of DonutOS.exe (WebView2 children, workers),
# per-process RSS / private bytes / handles / CPU time, TCP connection counts,
# and log counters (lines, WARN, network-ish patterns, retries).
# Usage: powershell -File perf-snapshot.ps1 -Base <install-dir> -Label t1 -OutDir <dir>
# Output: <OutDir>\<Label>.json plus a one-line summary on stdout.
param(
  [Parameter(Position = 0)][string]$Label = 'snap',
  [string]$Base = 'C:\Users\max\Desktop\DonutOS',
  [string]$OutDir = ''
)
$ErrorActionPreference = 'SilentlyContinue'
if (-not $OutDir) { $OutDir = Join-Path $Base 'logs' }
$Base = [System.IO.Path]::GetFullPath($Base)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$logPath = Join-Path $Base 'logs\DonutBrowser.log'

# --- Process tree -----------------------------------------------------------
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, ExecutablePath
# Root lookup by name-filtered query with retries: the client-side filtered
# full enum intermittently misses a live instance on this host.
$root = $null
for ($i = 0; $i -lt 3 -and -not $root; $i++) {
  $root = Get-CimInstance Win32_Process -Filter "Name='DonutOS.exe'" |
    Where-Object { $_.ExecutablePath -like "$Base*" } | Select-Object -First 1
  if (-not $root) { Start-Sleep -Seconds 2 }
}
if (-not $root) {
  @{ label = $Label; ts = (Get-Date -Format s); error = 'DonutOS.exe not found under Base' } |
    ConvertTo-Json -Depth 4 | Set-Content (Join-Path $OutDir "$Label.json") -Encoding UTF8
  Write-Output "SNAPSHOT $Label ERROR: DonutOS.exe not running under $Base"
  exit 1
}
$childrenOf = @{}
foreach ($p in $all) {
  if (-not $childrenOf.ContainsKey([string]$p.ParentProcessId)) { $childrenOf[[string]$p.ParentProcessId] = @() }
  $childrenOf[[string]$p.ParentProcessId] += $p
}
$tree = New-Object System.Collections.Generic.List[object]
$queue = New-Object System.Collections.Generic.Queue[object]
$queue.Enqueue($root)
$depth = @{ $root.ProcessId = 0 }
while ($queue.Count) {
  $cur = $queue.Dequeue()
  $tree.Add($cur)
  foreach ($c in @($childrenOf[[string]$cur.ProcessId])) {
    $depth[$c.ProcessId] = $depth[$cur.ProcessId] + 1
    $queue.Enqueue($c)
  }
}
$pids = @($tree | ForEach-Object { $_.ProcessId })

$procs = foreach ($t in $tree) {
  $gp = Get-Process -Id $t.ProcessId
  $tcpEst = @(Get-NetTCPConnection -OwningProcess $t.ProcessId -State Established).Count
  [ordered]@{
    pid       = $t.ProcessId
    name      = $t.Name
    main      = ($t.ProcessId -eq $root.ProcessId)
    kind      = if ($t.Name -eq 'DonutOS.exe' -and -not ($t.ProcessId -eq $root.ProcessId)) { 'sibling' }
                elseif ($t.Name -like 'msedgewebview2*') { 'webview2' }
                elseif ($depth[$t.ProcessId] -le 1) { 'child' } else { 'grandchild+' }
    rss_mb    = [math]::Round($gp.WorkingSet64 / 1MB, 1)
    priv_mb   = [math]::Round($gp.PrivateMemorySize64 / 1MB, 1)
    handles   = $gp.HandleCount
    cpu_s     = [math]::Round($gp.TotalProcessorTime.TotalSeconds, 1)
    tcp_est   = $tcpEst
  }
}

# --- Log counters -----------------------------------------------------------
$logInfo = [ordered]@{ path = $logPath }
if (Test-Path $logPath) {
  $lines = Get-Content $logPath
  $logInfo.lines = $lines.Count
  $logInfo.bytes = (Get-Item $logPath).Length
  $logInfo.warn = @($lines | Where-Object { $_ -match '\[WARN\]' }).Count
  $logInfo.error = @($lines | Where-Object { $_ -match '\[ERROR\]' }).Count
  # Network-facing activity markers (each category counted separately so the
  # report can tell retries apart from plain chatter).
  $logInfo.net_405 = @($lines | Where-Object { $_ -match 'status=405' }).Count
  $logInfo.net_remote_retry = @($lines | Where-Object { $_ -match 'Remote session stream' }).Count
  $logInfo.net_engine_fetch = @($lines | Where-Object { $_ -match 'engine|version check|Unsupported browser' -and $_ -match 'fetch|check|Unsupported' }).Count
  $logInfo.mcp_started = @($lines | Where-Object { $_ -match 'MCP session started' }).Count
  $logInfo.mcp_ended = @($lines | Where-Object { $_ -match 'MCP session (ended|closed)' }).Count
  $logInfo.info_lines = @($lines | Where-Object { $_ -match '\[INFO\]' }).Count
}

# --- Totals -----------------------------------------------------------------
$webview = @($procs | Where-Object { $_.kind -eq 'webview2' })
$snap = [ordered]@{
  label     = $Label
  ts        = (Get-Date -Format s)
  base      = $Base
  root_pid  = $root.ProcessId
  proc_count = $procs.Count
  webview_count = $webview.Count
  rss_total_mb = [math]::Round((($procs | Measure-Object rss_mb -Sum).Sum), 1)
  rss_main_mb = [math]::Round(($procs | Where-Object main).rss_mb, 1)
  rss_webview_mb = [math]::Round((($webview | Measure-Object rss_mb -Sum).Sum), 1)
  priv_total_mb = [math]::Round((($procs | Measure-Object priv_mb -Sum).Sum), 1)
  handles_total = ($procs | Measure-Object handles -Sum).Sum
  cpu_total_s = [math]::Round((($procs | Measure-Object cpu_s -Sum).Sum), 1)
  tcp_est_total = ($procs | Measure-Object tcp_est -Sum).Sum
  log = $logInfo
  procs = $procs
}
$json = $snap | ConvertTo-Json -Depth 6
Set-Content -Path (Join-Path $OutDir "$Label.json") -Value $json -Encoding UTF8
Write-Output ("SNAPSHOT {0} ts={1} procs={2} webview={3} rss={4}MB (main={5} wv={6}) priv={7}MB handles={8} cpu={9}s tcp={10} log_lines={11} warn={12}" -f `
  $Label, $snap.ts, $snap.proc_count, $snap.webview_count, $snap.rss_total_mb, $snap.rss_main_mb, $snap.rss_webview_mb, $snap.priv_total_mb, $snap.handlers_total, $snap.cpu_total_s, $snap.tcp_est_total, $logInfo.lines, $logInfo.warn)
