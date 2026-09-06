# Zyplus installer:  irm https://raw.githubusercontent.com/Fankrits/zyplus-editor/main/install.ps1 | iex
# Or pin a version:  $env:ZYPLUS_VERSION = '0.1.0'
$ErrorActionPreference = 'Stop'

$repo = 'Fankrits/zyplus-editor'
$version = $env:ZYPLUS_VERSION
if (-not $version) {
  $version = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name -replace '^v', ''
}
if (-not $version) { throw 'could not determine latest version' }

if ($env:PROCESSOR_ARCHITECTURE -notin 'AMD64', 'x86') {
  Write-Warning 'Only x64 Windows builds are published; running under emulation.'
}

$exe = Join-Path $env:TEMP "Zyplus_${version}_x64-setup.exe"
Write-Host "Downloading Zyplus $version"
Invoke-WebRequest "https://github.com/$repo/releases/download/v$version/Zyplus_${version}_x64-setup.exe" -OutFile $exe

# /S is the NSIS silent flag; Tauri's installer defaults to per-user install.
Start-Process $exe -ArgumentList '/S' -Wait
Remove-Item $exe -Force
Write-Host "Installed Zyplus $version."
