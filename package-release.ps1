param(
    [switch]$SkipMsi
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

function Resolve-CargoVersion {
    $cargoToml = Join-Path $Root "Cargo.toml"
    if (Test-Path -LiteralPath $cargoToml) {
        $match = Select-String -Path $cargoToml -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
        if ($match) {
            return $match.Matches[0].Groups[1].Value
        }
    }
    return "0.1.0"
}

# Stop existing SoundCast instances before overwriting dist/SoundCast.exe
Get-Process SoundCast -ErrorAction SilentlyContinue | Stop-Process -Force

# Prefer MSVC toolchain for clean manifest + icon resource merging
$toolchains = rustup toolchain list 2>$null
$useMsvc = $toolchains -match "stable-x86_64-pc-windows-msvc"

Write-Host "[SoundCast] Building release binary..." -ForegroundColor Cyan
if ($useMsvc) {
    cargo +stable-x86_64-pc-windows-msvc build --release --target x86_64-pc-windows-msvc
    $releaseExe = Join-Path $Root "target\x86_64-pc-windows-msvc\release\SoundCast.exe"
} else {
    cargo build --release
    $releaseExe = Join-Path $Root "target\release\SoundCast.exe"
}

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if (-not (Test-Path -LiteralPath $releaseExe)) {
    throw "Release executable not found: $releaseExe"
}

$version = Resolve-CargoVersion
$distDir = Join-Path $Root "dist"
if (Test-Path -LiteralPath $distDir) {
    Remove-Item -LiteralPath $distDir -Recurse -Force
}
$stageDir = Join-Path $distDir "stage"
New-Item -ItemType Directory -Force -Path $distDir, $stageDir | Out-Null

Copy-Item $releaseExe (Join-Path $stageDir "SoundCast.exe") -Force
Copy-Item (Join-Path $Root "README.md") (Join-Path $stageDir "README.md") -Force
Copy-Item (Join-Path $Root "LICENSE") (Join-Path $stageDir "LICENSE") -Force
Copy-Item $releaseExe (Join-Path $distDir "SoundCast.exe") -Force

$zipName = "SoundCast_v${version}.zip"
$zipPath = Join-Path $distDir $zipName
Compress-Archive `
    -Path (Join-Path $stageDir "*") `
    -DestinationPath $zipPath `
    -CompressionLevel Optimal `
    -Force

if (-not $SkipMsi) {
    $hasWix = (Get-Command wix -ErrorAction SilentlyContinue) -or (Test-Path -LiteralPath (Join-Path $env:USERPROFILE ".dotnet\tools\wix.exe"))
    if ($hasWix) {
        $msiName = "SoundCast_v${version}.msi"
        & (Join-Path $Root "installer\build_msi.ps1") `
            -SourceDir $stageDir `
            -OutputDir $distDir `
            -Version $version `
            -OutMsiName $msiName
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    } else {
        Write-Warning "[SoundCast] WiX CLI (wix.exe) not found, skipping .msi installer. Install with: dotnet tool install --global wix --version 5.0.2"
    }
}

Remove-Item -LiteralPath $stageDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "[SoundCast] Packages ready in dist/:" -ForegroundColor Green
Get-ChildItem -File $distDir | Select-Object Name, @{Name="SizeKB";Expression={[math]::Round($_.Length / 1KB, 1)}} | Format-Table -AutoSize
