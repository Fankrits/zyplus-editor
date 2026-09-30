# Zyplus installer:  irm https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.ps1 | iex
# Or pin a version:  $env:ZYPLUS_VERSION = '0.1.0'
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 may not offer TLS 1.2, which GitHub requires; and its
# progress bar slows Invoke-WebRequest down by an order of magnitude.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

$repo = 'Fankrits/zyplus-editor'
$api = if ($env:ZYPLUS_VERSION) {
  "https://api.github.com/repos/$repo/releases/tags/v$env:ZYPLUS_VERSION"
} else {
  "https://api.github.com/repos/$repo/releases/latest"
}

try { $release = Invoke-RestMethod $api } catch {
  throw "No release found at $api - check ZYPLUS_VERSION, or see https://github.com/$repo/releases"
}
$version = $release.tag_name -replace '^v', ''

# A 32-bit PowerShell on a 64-bit machine reports x86 here; the real machine
# architecture is then in PROCESSOR_ARCHITEW6432.
$machine = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }

# Match the published asset rather than guessing its filename.
$arch = if ($machine -eq 'ARM64') { 'arm64|aarch64' } else { 'x64|x86_64|amd64' }
$asset = $release.assets | Where-Object { $_.name -match "($arch).*-setup\.exe$" } | Select-Object -First 1

if (-not $asset -and $machine -eq 'ARM64') {
  # No native arm64 build; Windows runs the x64 installer under emulation.
  $asset = $release.assets | Where-Object { $_.name -match 'x64.*-setup\.exe$' } | Select-Object -First 1
  if ($asset) { Write-Warning 'No arm64 build published; installing the x64 build under emulation.' }
}
if (-not $asset) { throw "No Zyplus $version installer published for $machine. See https://github.com/$repo/releases" }

$exe = Join-Path $env:TEMP $asset.name
Write-Host "Downloading Zyplus $version"
Invoke-WebRequest $asset.browser_download_url -OutFile $exe

# The build is unsigned, so the download carries a Mark-of-the-Web that makes
# SmartScreen block the silent install outright. We fetched it from the release
# API ourselves, so clear it.
Unblock-File $exe

# /S is the NSIS silent flag; Tauri's installer defaults to a per-user install.
try {
  $p = Start-Process $exe -ArgumentList '/S' -Wait -PassThru
  if ($p.ExitCode -ne 0) { throw "Installer exited with code $($p.ExitCode)" }
} finally {
  # Also when Start-Process itself throws: the installer must not be left in TEMP.
  Remove-Item $exe -Force -ErrorAction SilentlyContinue
}
Write-Host "Installed Zyplus $version."
