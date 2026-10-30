# DonutOS perf snapshot: one JSON measurement point for a running instance.
# Captures the full process tree of DonutOS.exe (WebView2 children, workers),
# per-process RSS / private bytes / handles / CPU time, TCP connection counts,
# and log counters (lines, WARN, network-ish patterns, retries).
# Usage: powershell -File perf-snapshot.ps1 -Base <install-dir> -Label t1 -OutDir <dir>
# Output: <OutDir>\<Label>.json plus a one-line summary on stdout.
#
# Every section is fault-tolerant: a failure degrades that section (records it
# in `errors`) instead of losing the whole snapshot. The JSON is written even
# when collection dies mid-way -- two passes were burned by silent mid-body
# crashes before this hardening.
param(
  [Parameter(Position = 0)][string]$Label = 'snap',
  [string]$Base = 'C:\Users\max\Desktop\DonutOS',
  [string]$OutDir = ''
)
$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
$startedAt = Get-Date
if (-not $OutDir) { $OutDir = Join-Path $Base 'logs' }
$Base = [System.IO.Path]::GetFullPath($Base)
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$logPath = Join-Path $Base 'logs\DonutBrowser.log'
$errors = New-Object System.Collections.Generic.List[string]

# Collected progressively so a late failure still leaves the early parts.
$procs = @()
$logInfo = [ordered]@{ path = $logPath }
$outFile = Join-Path $OutDir "$Label.json"

# Written on every exit path, including throws: a missing file is a bug report,
# not a burned measurement pass.
function Write-Snap {
  param($Snap, $Target)
  try {
    $Snap.elapsed_s = [math]::Round(((Get-Date) - $script:startedAt).TotalSeconds, 1)
    $Snap | ConvertTo-Json -Depth 6 | Set-Content -Path $Target -Encoding UTF8
  } catch {
    @{ label = $Snap.label; ts = $Snap.ts; fatal = "$_" } |
      ConvertTo-Json -Depth 3 | Set-Content -Path $Target -Encoding UTF8
  }
}

try {
  # --- Root lookup (name-filtered query with retries) ------------------------
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, ExecutablePath
  $root = $null
  for ($i = 0; $i -lt 3 -and -not $root; $i++) {
    $root = Get-CimInstance Win32_Process -Filter "Name='DonutOS.exe'" |
      Where-Object { $_.ExecutablePath -like "$Base*" } | Select-Object -First 1
    if (-not $root) { Start-Sleep -Seconds 2 }
  }
  if (-not $root) {
    @{ label = $Label; ts = (Get-Date -Format s); error = 'DonutOS.exe not found under Base' } |
      ConvertTo-Json -Depth 4 | Set-Content $outFile -Encoding UTF8
    Write-Output "SNAPSHOT $Label ERROR: DonutOS.exe not running under $Base"
    exit 1
  }

  $snap = [ordered]@{
    label     = $Label
    ts        = (Get-Date -Format s)
    base      = $Base
    root_pid  = $root.ProcessId
  }

  # From here on, ANY failure still writes what exists.
  try {
    # --- Process tree (BFS, cycle-guarded against PID-reuse loops) -----------
    $childrenOf = @{}
    foreach ($p in $all) {
      if (-not $childrenOf.ContainsKey([string]$p.ParentProcessId)) { $childrenOf[[string]$p.ParentProcessId] = @() }
      $childrenOf[[string]$p.ParentProcessId] += $p
    }
    $tree = New-Object System.Collections.Generic.List[object]
    $queue = New-Object System.Collections.Generic.Queue[object]
    $queue.Enqueue($root)
    $depth = @{ $root.ProcessId = 0 }
    $seen = @{ $root.ProcessId = $true }
    while ($queue.Count) {
      $cur = $queue.Dequeue()
      $tree.Add($cur)
      foreach ($c in @($childrenOf[[string]$cur.ProcessId])) {
        if ($seen.ContainsKey($c.ProcessId)) { continue }
        $seen[$c.ProcessId] = $true
        $depth[$c.ProcessId] = $depth[$cur.ProcessId] + 1
        $queue.Enqueue($c)
      }
    }

    # --- Established-TCP counts per PID from ONE netstat call ----------------
    # Get-NetTCPConnection per PID can hang 40+ minutes on this host (WMI);
    # netstat parses the same table in under a second.
    $tcpByPid = @{}
    foreach ($line in (& netstat.exe -n -a -o -p tcp)) {
      if ($line -match 'ESTABLISHED\s+(\d+)\s*$') {
        $owner = $Matches[1]
        if ($tcpByPid.ContainsKey($owner)) { $tcpByPid[$owner] = $tcpByPid[$owner] + 1 }
        else { $tcpByPid[$owner] = 1 }
      }
    }

    # --- Per-process metrics, each failure isolated to its process ----------
    $procs = foreach ($t in $tree) {
      $row = [ordered]@{
        pid       = $t.ProcessId
        name      = $t.Name
        main      = ($t.ProcessId -eq $root.ProcessId)
        kind      = if ($t.Name -eq 'DonutOS.exe' -and -not ($t.ProcessId -eq $root.ProcessId)) { 'sibling' }
                    elseif ($t.Name -like 'msedgewebview2*') { 'webview2' }
                    elseif ($depth[$t.ProcessId] -le 1) { 'child' } else { 'grandchild+' }
        rss_mb    = -1; priv_mb = -1; handles = -1; cpu_s = -1
        tcp_est   = 0
        err       = $null
      }
      if ($tcpByPid.ContainsKey([string]$t.ProcessId)) { $row.tcp_est = [int]$tcpByPid[[string]$t.ProcessId] }
      try {
        $gp = Get-Process -Id $t.ProcessId -ErrorAction Stop
        $row.rss_mb  = [math]::Round($gp.WorkingSet64 / 1MB, 1)
        $row.priv_mb = [math]::Round($gp.PrivateMemorySize64 / 1MB, 1)
        $row.handles = $gp.HandleCount
        $row.cpu_s   = [math]::Round($gp.TotalProcessorTime.TotalSeconds, 1)
      } catch {
        $row.err = "get-process failed: $($_.Exception.Message)"
        $script:errors.Add("pid $($t.ProcessId): $($row.err)")
      }
      $row
    }
  } catch {
    $errors.Add("process section: $_")
  }

  # --- Log counters ----------------------------------------------------------
  try {
    if (Test-Path $logPath) {
      $lines = Get-Content $logPath -ErrorAction Stop
      $logInfo.lines = @($lines).Count
      $logInfo.bytes = (Get-Item $logPath).Length
      $logInfo.warn = @($lines | Where-Object { $_ -match '\[WARN\]' }).Count
      $logInfo.error = @($lines | Where-Object { $_ -match '\[ERROR\]' }).Count
      # Network-facing activity markers, each category counted separately so
      # the report can tell retries apart from plain chatter.
      $logInfo.net_405 = @($lines | Where-Object { $_ -match 'status=405' }).Count
      $logInfo.net_remote_retry = @($lines | Where-Object { $_ -match 'Remote session stream' }).Count
      $logInfo.net_engine_fetch = @($lines | Where-Object { $_ -match 'engine|version check|Unsupported browser' -and $_ -match 'fetch|check|Unsupported' }).Count
      $logInfo.mcp_started = @($lines | Where-Object { $_ -match 'MCP session started' }).Count
      $logInfo.mcp_ended = @($lines | Where-Object { $_ -match 'MCP session (ended|closed)' }).Count
      $logInfo.info_lines = @($lines | Where-Object { $_ -match '\[INFO\]' }).Count
    } else {
      $logInfo.missing = $true
    }
  } catch {
    $logInfo.error = "read failed: $($_.Exception.Message)"
    $errors.Add("log section: $_")
  }

  # --- Totals ----------------------------------------------------------------
  $plist = @($procs)
  $webview = @($plist | Where-Object { $_.kind -eq 'webview2' })
  $okRss = @($plist | Where-Object { $_.rss_mb -ge 0 })
  $snap.proc_count = $plist.Count
  $snap.webview_count = $webview.Count
  $snap.rss_total_mb = if ($okRss.Count) { [math]::Round((($okRss | Measure-Object rss_mb -Sum).Sum), 1) } else { $null }
  $snap.rss_main_mb = try { [math]::Round(($plist | Where-Object main).rss_mb, 1) } catch { $null }
  $snap.rss_webview_mb = if ($webview.Count) { [math]::Round((($webview | Measure-Object rss_mb -Sum).Sum), 1) } else { 0 }
  $snap.priv_total_mb = if ($okRss.Count) { [math]::Round((($okRss | Measure-Object priv_mb -Sum).Sum), 1) } else { $null }
  $snap.handles_total = ($plist | Measure-Object handles -Sum).Sum
  $snap.cpu_total_s = [math]::Round((($plist | Measure-Object cpu_s -Sum).Sum), 1)
  $snap.tcp_est_total = ($plist | Measure-Object tcp_est -Sum).Sum
  $snap.log = $logInfo
  $snap.procs = $plist
  if ($errors.Count) { $snap.errors = @($errors) }

  Write-Snap $snap $outFile
  Write-Output ("SNAPSHOT {0} ts={1} procs={2} webview={3} rss={4}MB (main={5} wv={6}) priv={7}MB handles={8} cpu={9}s tcp={10} log_lines={11} warn={12} errors={13} elapsed={14}s" -f `
    $Label, $snap.ts, $snap.proc_count, $snap.webview_count, $snap.rss_total_mb, $snap.rss_main_mb, $snap.rss_webview_mb, $snap.priv_total_mb, $snap.handles_total, $snap.cpu_total_s, $snap.tcp_est_total, $logInfo.lines, $logInfo.warn, $errors.Count, $snap.elapsed_s)
} catch {
  # Anything unexpected: still leave a file with what exists.
  $snap = [ordered]@{
    label = $Label; ts = (Get-Date -Format s); base = $Base
    fatal = "$_"
    log   = $logInfo
  }
  if ($procs) { $snap.procs = @($procs) }
  if ($errors.Count) { $snap.errors = @($errors) }
  Write-Snap $snap $outFile
  Write-Output "SNAPSHOT $Label PARTIAL: fatal=$($_.Exception.Message) errors=$($errors.Count)"
}
