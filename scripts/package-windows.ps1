# Build klipa.exe and the NSIS installer on Windows.
#
#   pwsh scripts/package-windows.ps1
#
# Requires: Rust (MSVC toolchain) and NSIS (makensis on PATH).
# Optionally signs with signtool if $env:WIN_CERT_PFX + $env:WIN_CERT_PASS
# are set (your own code-signing certificate).
#
# Output: dist/klipa-<version>-windows-x64-setup.exe
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' |
            Select-Object -First 1).Matches.Groups[1].Value

Write-Host "==> cargo build --release"
cargo build --release -p klipa-ui

New-Item -ItemType Directory -Force -Path dist | Out-Null
Copy-Item "target\release\klipa.exe" "dist\klipa.exe" -Force

if ($env:WIN_CERT_PFX) {
  Write-Host "==> signing klipa.exe"
  signtool sign /fd SHA256 /f $env:WIN_CERT_PFX /p $env:WIN_CERT_PASS `
    /tr http://timestamp.digicert.com /td SHA256 "dist\klipa.exe"
}

Write-Host "==> makensis"
makensis "/DVERSION=$version" "packaging\windows\klipa.nsi"

$setup = "dist\klipa-$version-windows-x64-setup.exe"
if ($env:WIN_CERT_PFX) {
  Write-Host "==> signing installer"
  signtool sign /fd SHA256 /f $env:WIN_CERT_PFX /p $env:WIN_CERT_PASS `
    /tr http://timestamp.digicert.com /td SHA256 $setup
}
Write-Host "Built $setup"
