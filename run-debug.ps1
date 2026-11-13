param(
    [switch]$SettingsOnly
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

# Stop existing SoundCast instances so the binary is not locked
Get-Process SoundCast -ErrorAction SilentlyContinue | Stop-Process -Force

# Prefer MSVC toolchain if installed
$toolchains = rustup toolchain list 2>$null
$useMsvc = $toolchains -match "stable-x86_64-pc-windows-msvc"

Write-Host "[SoundCast] Building debug binary..." -ForegroundColor Cyan
if ($useMsvc) {
    cargo +stable-x86_64-pc-windows-msvc build --bin SoundCast
} else {
    cargo build --bin SoundCast
}

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$exe = Join-Path $Root "target\debug\SoundCast.exe"
if (-not (Test-Path $exe)) {
    throw "Debug executable not found: $exe"
}

if ($SettingsOnly) {
    Write-Host "[SoundCast] Launching settings UI preview..." -ForegroundColor Green
    Start-Process $exe -ArgumentList "--settings"
} else {
    Write-Host "[SoundCast] Launching daemon and settings UI..." -ForegroundColor Green
    Start-Process $exe
    Start-Sleep -Milliseconds 600
    Start-Process $exe -ArgumentList "--settings"
}

