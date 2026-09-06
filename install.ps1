# Zyplus installer:  irm https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.ps1 | iex
# Or pin a version:  $env:ZYPLUS_VERSION = '0.1.0'
$ErrorActionPreference = 'Stop'

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

# Match the published asset rather than guessing its filename.
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64|aarch64' } else { 'x64|x86_64|amd64' }
$asset = $release.assets | Where-Object { $_.name -match "($arch).*-setup\.exe$" } | Select-Object -First 1

if (-not $asset -and $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
  # No native arm64 build; Windows runs the x64 installer under emulation.
  $asset = $release.assets | Where-Object { $_.name -match 'x64.*-setup\.exe$' } | Select-Object -First 1
  if ($asset) { Write-Warning 'No arm64 build published; installing the x64 build under emulation.' }
}
if (-not $asset) { throw "No Zyplus $version installer published for $env:PROCESSOR_ARCHITECTURE. See https://github.com/$repo/releases" }

$exe = Join-Path $env:TEMP $asset.name
Write-Host "Downloading Zyplus $version"
Invoke-WebRequest $asset.browser_download_url -OutFile $exe

# /S is the NSIS silent flag; Tauri's installer defaults to a per-user install.
$p = Start-Process $exe -ArgumentList '/S' -Wait -PassThru
Remove-Item $exe -Force
if ($p.ExitCode -ne 0) { throw "Installer exited with code $($p.ExitCode)" }
Write-Host "Installed Zyplus $version."
