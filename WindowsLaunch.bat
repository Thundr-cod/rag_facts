@echo off
:: facts. launcher for Windows. All logic lives in .system\facts-windows.exe (source: facts-rs\).
title facts.
chcp 65001 <nul >nul

:: Kill an engine left running by a crashed session (it would hold RAM and VRAM).
taskkill /F /IM llama-server.exe /T <nul >nul 2>&1

:: Remove the "downloaded from the internet" mark so SmartScreen doesn't block the unsigned files.
powershell -NoProfile -Command "Get-ChildItem -LiteralPath '%~dp0.system' -Recurse -File | Unblock-File" <nul >nul 2>&1

if not exist "%~dp0.system\facts-windows.exe" (
    echo.
    echo   [ERROR] .system\facts-windows.exe is missing.
    echo   Your antivirus may have removed it. Check Windows Security - Protection History,
    echo   restore the file and add an exclusion.
    echo   Need help? Visit opensourceeverything.io and use the support chat.
    echo.
    pause
    exit /b 1
)

cls
"%~dp0.system\facts-windows.exe"
pause
