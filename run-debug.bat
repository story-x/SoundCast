@echo off
cd /d "%~dp0"

where pwsh >nul 2>nul
if %errorlevel%==0 (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-debug.ps1" %*
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run-debug.ps1" %*
)

if %errorlevel% neq 0 (
    echo.
    echo [SoundCast] Failed with exit code %errorlevel%.
    pause
    exit /b %errorlevel%
)

