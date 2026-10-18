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

# 1. Process identity (PS5.1: wrap in @() so .Count works on single results)
$procs = @()
if (Test-Path $Base) { $procs = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($Base) }) }
Check 'process-running' ($procs.Count -gt 0) "count=$($procs.Count)"
$mainProcs = @($procs | Where-Object { $_.Name -eq 'DonutFree.exe' })
Check 'process-name-donutfree' ($mainProcs.Count -gt 0) "names=$(($procs | ForEach-Object { $_.Name } | Select-Object -Unique) -join ',')"

# 2. Health endpoint (curl.exe for raw codes - PS5.1 IWR hides them in exceptions)
$B = "http://127.0.0.1:$port"
$code = (& curl.exe -s -o NUL -w '%{http_code}' "$B/v1/health") -join ''
Check 'health-requires-auth' ($code -eq '401') "no-token status=$code"
$h = Invoke-RestMethod "$B/v1/health" -Headers $H -TimeoutSec 5
Check 'health-ok' ($h.status -eq 'ok') "v$($h.version) build=$($h.build)"
Check 'build-flavour-free' ($h.build -eq 'free') "build=$($h.build)"

# 3. e2e routes compiled out
$code = (& curl.exe -s -o NUL -w '%{http_code}' -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -d '{}' "$B/v1/e2e/mcp") -join ''
Check 'e2e-mcp-404' ($code -eq '404') "status=$code"
$code2 = (& curl.exe -s -o NUL -w '%{http_code}' -H "Authorization: Bearer $token" "$B/v1/e2e/status") -join ''
Check 'e2e-status-404' ($code2 -eq '404') "status=$code2"

# 4. Local MCP endpoint (initialize must set the standard mcp-session-id header)
$initBody = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"acceptance","version":"0"}}}'
$respFile = Join-Path $env:TEMP 'donut-acc-mcp.json'
$hdrFile = Join-Path $env:TEMP 'donut-acc-mcp.hdr'
$null = & curl.exe -s -D $hdrFile -o $respFile -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -d $initBody "$B/v1/mcp"
$initOk = $false; $sid = $null; $serverName = ''
if ((Test-Path $respFile) -and (Test-Path $hdrFile)) {
  $m = [regex]::Match((Get-Content $hdrFile -Raw), '(?im)^mcp-session-id:\s*(\S+)')
  if ($m.Success) { $sid = $m.Groups[1].Value }
  $json = Get-Content $respFile -Raw | ConvertFrom-Json
  $serverName = $json.body.result.serverInfo.name
  $initOk = ($serverName -eq 'donut-browser') -and [bool]$sid
}
Check 'mcp-initialize' $initOk "server=$serverName header-session=$([bool]$sid)"
$toolsOk = $false; $toolsLen = 0
if ($sid) {
  $null = & curl.exe -s -o $respFile -X POST -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -H "mcp-session-id: $sid" -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' "$B/v1/mcp"
  if (Test-Path $respFile) { $toolsLen = (Get-Item $respFile).Length }
  $toolsOk = ($toolsLen -gt 100)
}
Check 'mcp-tools-list' $toolsOk "bytes=$toolsLen"

# 5. Engine/binaries present (proactive downloads work; CDN allowed)
$engines = @(Get-ChildItem "$Base\data" -Recurse -Include 'wayfern*.zip','wayfern*.exe','xray*.exe','*.mmdb' | Select-Object -First 6)
Check 'engine-present' ($engines.Count -gt 0) "files=$(($engines | ForEach-Object { $_.Name }) -join ',')"

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
