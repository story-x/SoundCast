param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDir,
    [Parameter(Mandatory = $true)]
    [string]$OutputDir,
    [string]$Version = "",
    [string]$OutMsiName = "SoundCast.msi",
    [string]$IconPath = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot

function Resolve-CargoVersion {
    $cargoToml = Join-Path $RepoRoot "Cargo.toml"
    if (Test-Path -LiteralPath $cargoToml) {
        $match = Select-String -Path $cargoToml -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
        if ($match) {
            return $match.Matches[0].Groups[1].Value
        }
    }
    return "0.1.0"
}

function Normalize-MsiVersion([string]$RawVersion) {
    $candidate = $RawVersion
    if (-not [string]::IsNullOrWhiteSpace($candidate) -and $candidate -match '^v?(\d+\.\d+\.\d+)(?:\.(\d+))?') {
        if ($Matches[2]) {
            return "$($Matches[1]).$($Matches[2])"
        }
        return "$($Matches[1]).0"
    }
    $fallback = Resolve-CargoVersion
    if ($fallback -match '^v?(\d+\.\d+\.\d+)(?:\.(\d+))?') {
        if ($Matches[2]) {
            return "$($Matches[1]).$($Matches[2])"
        }
        return "$($Matches[1]).0"
    }
    return "0.1.0.0"
}

function Resolve-WixExe {
    $cmd = Get-Command wix -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }
    $dotnetToolWix = Join-Path $env:USERPROFILE ".dotnet\tools\wix.exe"
    if (Test-Path -LiteralPath $dotnetToolWix) {
        return $dotnetToolWix
    }
    throw "WiX CLI (wix.exe) not found. Install it with: dotnet tool install --global wix --version 5.0.2"
}

$resolvedSource = (Resolve-Path -LiteralPath $SourceDir).Path
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$resolvedOutputDir = (Resolve-Path -LiteralPath $OutputDir).Path

if ([string]::IsNullOrWhiteSpace($IconPath)) {
    $IconPath = Join-Path $RepoRoot "assets\icon.ico"
}
$resolvedIcon = (Resolve-Path -LiteralPath $IconPath).Path
$resolvedLicenseRtf = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "license.rtf")).Path
$wxsFile = Join-Path $PSScriptRoot "SoundCast.wxs"
$msiVersion = Normalize-MsiVersion $Version
$outMsiPath = Join-Path $resolvedOutputDir $OutMsiName
$wixExe = Resolve-WixExe

Write-Host "[SoundCast] Building MSI ($msiVersion) -> $outMsiPath" -ForegroundColor Cyan

& $wixExe build $wxsFile `
    -arch x64 `
    -ext WixToolset.UI.wixext `
    -culture zh-CN `
    -d "SourceDir=$resolvedSource" `
    -d "IconPath=$resolvedIcon" `
    -d "LicenseRtfPath=$resolvedLicenseRtf" `
    -d "ProductVersion=$msiVersion" `
    -o $outMsiPath

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if (-not (Test-Path -LiteralPath $outMsiPath)) {
    throw "MSI build did not produce output file: $outMsiPath"
}

$wixPdbPath = [System.IO.Path]::ChangeExtension($outMsiPath, ".wixpdb")
if (Test-Path -LiteralPath $wixPdbPath) {
    Remove-Item -LiteralPath $wixPdbPath -Force -ErrorAction SilentlyContinue
}

Write-Host "[SoundCast] MSI created: $outMsiPath" -ForegroundColor Green

