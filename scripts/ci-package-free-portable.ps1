# Packages the DonutFree portable zip for CI. Mirrors scripts/deploy-free.cmd:
# raw Tauri output plus sidecars, resources, the .portable marker, pre-seeded
# settings/token and the ctl scripts. The Wayfern engine is deliberately not
# bundled: free builds keep proactive engine download enabled, so a fresh
# install fetches engine and GeoIP from the vendor CDN on first start.
param(
  [string]$Root = ""
)

$ErrorActionPreference = "Stop"

if (-not $Root) { $Root = Split-Path -Parent $PSScriptRoot }
Set-Location $Root

$conf = Get-Content "src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version
$rel = "src-tauri/target/release"

$rustTarget = "x86_64-pc-windows-msvc"
try {
  $hostLine = (& rustc -vV) | Where-Object { $_ -match "^host:" } |
    Select-Object -First 1
  if ($hostLine -match "^host:\s*(\S+)") { $rustTarget = $Matches[1] }
} catch {}

function Require([string]$Path, [string]$What) {
  if (-not (Test-Path $Path)) {
    throw "portable package incomplete: $What missing at $Path"
  }
}

# A tag build must agree with tauri.conf.json, otherwise the zip's embedded
# version would disagree with its release tag.
$ref = $env:FREE_PORTABLE_REF
if ($ref -like "refs/tags/*") {
  $tag = $ref -replace "^refs/tags/", ""
  if ($tag -notlike "v$version-free*") {
    throw "tag '$tag' does not match app version '$version' (expected v$version-free*)"
  }
}

if (Test-Path "dist/portable") { Remove-Item -Recurse -Force "dist/portable" }
$stage = "dist/portable/DonutFree-$version-portable"
New-Item -ItemType Directory -Force -Path $stage | Out-Null

# The cargo binary keeps the package name; ship it as DonutFree.exe.
Require "$rel/donutbrowser.exe" "tauri build output"
Copy-Item "$rel/donutbrowser.exe" (Join-Path $stage "DonutFree.exe")

# Sidecars: the app bundler copies them next to the exe without the target
# suffix; fall back to the src-tauri/binaries staging copies.
foreach ($pair in @(
    @{ dest = "donut-proxy.exe"; built = "$rel/donut-proxy.exe"; staged = "src-tauri/binaries/donut-proxy-$rustTarget.exe" },
    @{ dest = "xray.exe"; built = "$rel/xray.exe"; staged = "src-tauri/binaries/xray-$rustTarget.exe" }
  )) {
  if (Test-Path $pair.built) {
    Copy-Item $pair.built (Join-Path $stage $pair.dest)
  } elseif (Test-Path $pair.staged) {
    Copy-Item $pair.staged (Join-Path $stage $pair.dest)
  } else {
    throw "portable package incomplete: $($pair.dest) not found in target or binaries staging"
  }
}
if (Test-Path "$rel/donut-xray.exe") {
  Copy-Item "$rel/donut-xray.exe" (Join-Path $stage "donut-xray.exe")
}

# tauri.conf maps binaries/xray-LICENSE.txt -> licenses/Xray-core-LICENSE.txt.
# Windows CI builds do not stage target/release/resources (only installers embed
# them), so fall back to the download-xray.mjs staging copy of the license.
$targetResources = "$rel/resources"
$targetLicense = "$targetResources/licenses/Xray-core-LICENSE.txt"
$stagedLicense = "src-tauri/binaries/xray-LICENSE.txt"
if (Test-Path $targetLicense) {
  Copy-Item -Recurse $targetResources (Join-Path $stage "resources")
} elseif (Test-Path $stagedLicense) {
  New-Item -ItemType Directory -Force -Path (Join-Path $stage "resources/licenses") | Out-Null
  Copy-Item $stagedLicense (Join-Path $stage "resources/licenses/Xray-core-LICENSE.txt")
} else {
  throw "portable package incomplete: Xray-core license not found at $targetLicense or $stagedLicense"
}

# .portable keeps all data next to the exe instead of %LOCALAPPDATA%.
Set-Content -Path (Join-Path $stage ".portable") -Value "" -NoNewline

# Pre-seed settings: local API on, port 10109 (an upstream install owns 10108).
New-Item -ItemType Directory -Force -Path "$stage/data/settings" | Out-Null
Set-Content -Path "$stage/data/settings/app_settings.json" -Value '{"api_enabled": true, "api_port": 10109}' -NoNewline

# Per-build automation token: ctl reads it and pins it via
# DONUT_AUTOMATION_TOKEN at start. Replace the file to rotate.
New-Item -ItemType Directory -Force -Path "$stage/config" | Out-Null
$token = ([guid]::NewGuid().ToString("N")) + ([guid]::NewGuid().ToString("N"))
Set-Content -Path "$stage/config/automation-token.txt" -Value $token -NoNewline

Copy-Item "scripts/donut-freectl.cmd" $stage
Copy-Item "scripts/donut-freectl.ps1" $stage
Copy-Item "LICENSE" (Join-Path $stage "LICENSE.txt")

$commit = "unknown"
try { $commit = (& git rev-parse --short HEAD 2>$null).Trim() } catch {}
$stamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd HH:mm 'UTC'")
Set-Content -Path (Join-Path $stage "version.txt") -Value "donut-free $version built $stamp commit $commit"

New-Item -ItemType Directory -Force -Path "dist" | Out-Null
$zip = "dist/DonutFree-$version-windows-x64-portable.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
# bsdtar (System32\tar.exe) instead of Compress-Archive: the latter skips
# dot-files, which would silently drop the .portable marker. Absolute path:
# a tar.exe earlier in PATH (e.g. an MSYS GNU tar) cannot write zips and
# would exit 0 with a non-zip file.
$tarExe = Join-Path $env:SystemRoot "System32\tar.exe"
if (-not (Test-Path $tarExe)) { $tarExe = "tar.exe" }
& $tarExe -a -c -f $zip -C "dist/portable" "DonutFree-$version-portable"
if ($LASTEXITCODE -ne 0) { throw "tar failed to create the portable zip" }

# Self-sufficiency gate: the zip must carry everything a cold start needs
# except the engine, which is downloaded on demand.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $zip).Path)
try {
  $entries = @($archive.Entries | ForEach-Object { $_.FullName })
  $prefix = "DonutFree-$version-portable"
  foreach ($required in @(
      "DonutFree.exe", "donut-proxy.exe", "xray.exe", ".portable",
      "data/settings/app_settings.json", "config/automation-token.txt",
      "donut-freectl.cmd", "donut-freectl.ps1", "version.txt", "LICENSE.txt",
      "resources/licenses/Xray-core-LICENSE.txt"
    )) {
    if ($entries -notcontains "$prefix/$required") {
      throw "zip self-check failed: '$required' missing from $zip"
    }
  }
} finally {
  $archive.Dispose()
}

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
$zipName = Split-Path -Leaf $zip
Set-Content -Path "dist/SHA256SUMS.txt" -Value "$hash  $zipName" -NoNewline

$sourceUrl = "$env:GITHUB_SERVER_URL/$env:GITHUB_REPOSITORY"
$notes = @"
DonutFree $version - Windows x64 portable build.

- Unzip anywhere, then run ``donut-freectl.cmd start`` (or launch ``DonutFree.exe``). No installer, no admin rights.
- The Wayfern browser engine and GeoIP data are downloaded automatically on first start - the zip never bundles the engine.
- Local REST/MCP API: ``http://127.0.0.1:10109`` (``GET /v1/health``); token in ``config\automation-token.txt`` (``donut-freectl.cmd token``; replace the file to rotate).
- All data stays next to the exe (``.portable``): copy the folder to move the install. Runs side by side with the original Donut Browser.
- Source (AGPL-3.0): $sourceUrl
- SHA256 (``$zipName``): ``$hash``
"@
Set-Content -Path "dist/RELEASE-NOTES.md" -Value $notes

Write-Host "PACKAGE_OK $zipName"
Write-Host "SHA256 $hash"
