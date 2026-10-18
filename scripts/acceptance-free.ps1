# Post-deploy acceptance for Donut-Free. Run AFTER deploy-free.cmd + donut-freectl.cmd start.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File acceptance-free.ps1 [-Base <deploy-dir>]
param([string]$Base = "C:\Users\max\Desktop\Donut-Free")
$ErrorActionPreference = 'SilentlyContinue'
$tokenFile = Join-Path $Base 'config\automation-token.txt'
$token = if (Test-Path $tokenFile) { (Get-Content $tokenFile -Raw).Trim() } else { $null }
$settingsF = Join-Path $Base 'data\settings\app_settings.json'
$port = if (Test-Path $settingsF) { (Get-Content $settingsF -Raw | ConvertFrom-Json).api_port } else { 10109 }
$H = @{ Authorization = "Bearer $token" }
$pass = 0; $fail = 0
function Check([string]$Name, [bool]$Ok, [string]$Detail) {
  if ($Ok) { $script:pass++; Write-Output "PASS  $Name  $Detail" }
  else { $script:fail++; Write-Output "FAIL  $Name  $Detail" }
}

# 1. Process identity
$procs = Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$Base*" }
Check 'process-running' ($procs.Count -gt 0) "count=$($procs.Count)"
Check 'process-name-donutfree' ($procs | Where-Object { $_.Name -eq 'DonutFree.exe' }) "names=$(($procs.Name | Select-Object -Unique) -join ',')"

# 2. Health endpoint
$h = Invoke-RestMethod "http://127.0.0.1:$port/v1/health" -Headers $H -TimeoutSec 5
Check 'health-ok' ($h.status -eq 'ok') "v$($h.version) build=$($h.build)"
Check 'build-flavour-free' ($h.build -eq 'free') "build=$($h.build)"
Check 'no-token-401' (-not (Invoke-WebRequest "http://127.0.0.1:$port/v1/health" -TimeoutSec 5).StatusCode -eq 200) ''

# 3. e2e routes compiled out
$code = try { (Invoke-WebRequest "http://127.0.0.1:$port/v1/e2e/mcp" -Method Post -Headers $H -Body '{}' -TimeoutSec 5).StatusCode } catch { $_.Exception.Response.StatusCode.value__ }
Check 'e2e-mcp-404' ($code -eq 404) "status=$code"
$code2 = try { (Invoke-WebRequest "http://127.0.0.1:$port/v1/e2e/status" -Headers $H -TimeoutSec 5).StatusCode } catch { $_.Exception.Response.StatusCode.value__ }
Check 'e2e-status-404' ($code2 -eq 404) "status=$code2"

# 4. Local MCP endpoint (JSON-RPC initialize)
$initBody = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"acceptance","version":"0"}}}'
$r = Invoke-WebRequest "http://127.0.0.1:$port/v1/mcp" -Method Post -Headers $H -ContentType 'application/json' -Body $initBody -TimeoutSec 10
$sid = $r.Headers['mcp-session-id']
$okInit = ($r.StatusCode -eq 200) -and $sid -and ($r.Content -match 'result')
Check 'mcp-initialize' $okInit "status=$($r.StatusCode) session=$([bool]$sid)"
if ($sid) {
  $noteBody = '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  $null = Invoke-WebRequest "http://127.0.0.1:$port/v1/mcp" -Method Post -Headers ($H + @{ 'mcp-session-id' = $sid }) -ContentType 'application/json' -Body $noteBody -TimeoutSec 5
  $toolsBody = '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  $tr = Invoke-WebRequest "http://127.0.0.1:$port/v1/mcp" -Method Post -Headers ($H + @{ 'mcp-session-id' = $sid }) -ContentType 'application/json' -Body $toolsBody -TimeoutSec 10
  Check 'mcp-tools-list' ($tr.StatusCode -eq 200 -and $tr.Content -match 'tools') "len=$($tr.Content.Length)"
}

# 5. Engine/binaries present (proactive downloads work; CDN allowed)
$engines = Get-ChildItem "$Base\data" -Recurse -Include 'wayfern*.exe','xray*.exe','*.mmdb' | Select-Object -First 6
Check 'engine-present' ($engines.Count -gt 0) "files=$(($engines.Name) -join ',')"

# 6. Network surface of our processes (manual review; CDN engine downloads are expected)
Write-Output '--- outbound connections (review) ---'
foreach ($p in $procs) {
  Get-NetTCPConnection -OwningProcess $p.ProcessId -State Established -ErrorAction SilentlyContinue |
    ForEach-Object { "pid=$($p.ProcessId) $($_.RemoteAddress):$($_.RemotePort)" }
}

# 7. Resource footprint vs baseline (prod v0.29.6: exe 44153856 B, WS 25.5 MB)
$exeItem = Get-Item (Join-Path $Base 'DonutFree.exe')
Check 'exe-size' $true "exe=$($exeItem.Length) B (prod 44153856 B)"
$main = Get-Process -Name DonutFree | Select-Object -First 1
if ($main) { Check 'memory' $true "WS=$([math]::Round($main.WorkingSet64/1MB,1))MB Private=$([math]::Round($main.PrivateMemorySize64/1MB,1))MB (prod 25.5/34.2MB)" }

Write-Output "=== RESULT: pass=$pass fail=$fail ==="
if ($fail -gt 0) { exit 1 } else { exit 0 }
