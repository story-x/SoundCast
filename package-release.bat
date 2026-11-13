@echo off
cd /d "%~dp0"

where pwsh >nul 2>nul
if %errorlevel%==0 (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0package-release.ps1"
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0package-release.ps1"
)

echo.
if %errorlevel% neq 0 (
    echo [SoundCast] Package failed with exit code %errorlevel%.
) else (
    echo [SoundCast] Package finished.
)
pause

