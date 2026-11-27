@echo off
setlocal enabledelayedexpansion
:: ==============================================================================
::  WindowsLaunch.bat
::  Engine: Native llama-cli.exe (Vulkan GPU - works with NVIDIA, AMD, Intel)
::  Logic: Kill Ghosts | Wipe History | Smart Model | GPU Detect | No Logs
:: ==============================================================================

:: Sets the window title
title Qwen AI - Windows Launcher

:: 1. KILL GHOST PROCESSES
taskkill /F /IM llama-cli.exe /T >nul 2>&1

:: 2. DEFINE PATHS
set "ROOT_DIR=%~dp0"
set "SYSTEM_DIR=%ROOT_DIR%.system"
set "WIN_DIR=%SYSTEM_DIR%\windows"
set "BINARY=%WIN_DIR%\llama-cli.exe"

set "MODEL_HIGH=%SYSTEM_DIR%\Qwen3.5-4B-abliterated.Q8_0.gguf"
set "MODEL_LOW=%SYSTEM_DIR%\Qwen3.5-4B-abliterated.Q4_K_M.gguf"
:: Different drive versions ship different model filenames. If this
:: launcher's names aren't on the drive, find whatever Qwen build is.
if not exist "%MODEL_HIGH%" if not exist "%MODEL_LOW%" (
    for %%F in ("%SYSTEM_DIR%\Qwen*.Q8_0.gguf") do set "MODEL_HIGH=%%~fF"
    for %%F in ("%SYSTEM_DIR%\Qwen*.Q4_K_M.gguf") do set "MODEL_LOW=%%~fF"
)

:: 3. PRE-FLIGHT CHECK
if not exist "%BINARY%" (
    echo.
    echo   [ERROR] llama-cli.exe not found in .system\windows\
    echo   The AI engine is missing. Your drive may be corrupted
    echo   or your antivirus may have deleted it.
    echo.
    echo   Check Windows Security - Protection History for blocked files.
    echo   If the file was quarantined, restore it and add an exclusion.
    echo.
    echo   Need help? Visit opensourceeverything.io and use the support chat.
    echo.
    pause
    exit
)

cls
chcp 65001 >nul
type "%SYSTEM_DIR%\ose-logo-win.txt" 2>nul
echo ----------------------------------------------------------------
echo   INITIALIZING QWEN AI [WINDOWS]...
echo ----------------------------------------------------------------

:: 4. MEMORY WIPE (The "Zero-Log" Feature)
if exist "%USERPROFILE%\.llama_history" del /f /q "%USERPROFILE%\.llama_history"
if exist "%ROOT_DIR%llama.chat.history" del /f /q "%ROOT_DIR%llama.chat.history"
if exist "%ROOT_DIR%main.session" del /f /q "%ROOT_DIR%main.session"
REM SYSTEM_DIR has NO trailing backslash, so these need an explicit \ — without it the paths
REM were "%SYSTEM_DIR%llama.chat.history" (never exist) and the .system copies were never
REM wiped, silently breaking the zero-log claim on Windows. (memory 32 W1)
if exist "%SYSTEM_DIR%\llama.chat.history" del /f /q "%SYSTEM_DIR%\llama.chat.history"
if exist "%SYSTEM_DIR%\main.session" del /f /q "%SYSTEM_DIR%\main.session"

echo   Cache Status: Wiped Clean [Zero-Log Mode]

:: 5. HARDWARE DETECTION (RAM)
for /f "tokens=*" %%g in ('powershell -command "[Math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)"') do set RAM_GB=%%g

echo   Hardware Detected: !RAM_GB! GB RAM

:: 6. AVAILABLE RAM CHECK
for /f "tokens=*" %%g in ('powershell -command "[Math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB)"') do set AVAIL_GB=%%g

echo   Available RAM: !AVAIL_GB! GB

if !AVAIL_GB! LSS 4 (
    echo.
    echo   [WARNING] Low available RAM. Close other apps for best performance.
    echo   The AI needs at least 4 GB free to run smoothly.
    echo   Close browsers, Discord, and other heavy apps, then try again.
    echo.
)

:: 7. MODEL + DEVICE PICK
:: pick-model.ps1 looks at every .gguf in .system, keeps the ones that fit this PC, benchmarks a
:: few (GPU vs CPU) and picks the biggest one that still runs at a usable speed. The pick is
:: cached per computer, so moving the drive to a new PC re-tests it.
set "GPU_FLAGS="

:: Prevent known Vulkan driver hangs
set "GGML_VK_DISABLE_COOPMAT=1"
set "VK_LOADER_LAYERS_DISABLE=~implicit~"

set "CTX_SIZE=8192"
REM Context sized to RAM — KV for the stock 4B model ~144KB/token (16K~2.3GB, 32K~4.6GB). (memory 32 C1)
REM Tiers are nominal-minus-1 (15/30): an integrated-GPU 16GB box reports ~15GB after the
REM hardware-reserved carve-out, so GEQ 16 would miss it. (memory 32 C2)
if !RAM_GB! GEQ 30 (set "CTX_SIZE=32768") else if !RAM_GB! GEQ 15 (set "CTX_SIZE=16384")

echo   Choosing the best model for this PC...
echo   [First launch on a new PC benchmarks each model, ~30-90 sec each. It is not frozen.]
set "PICK="
for /f "tokens=*" %%r in ('powershell -NoProfile -ExecutionPolicy Bypass -File "%WIN_DIR%\pick-model.ps1" "%SYSTEM_DIR%" !CTX_SIZE!') do set "PICK=%%r"
for /f "tokens=1-4 delims=|" %%a in ("!PICK!") do (
    set "PICK_STATUS=%%a"
    set "SELECTED_MODEL=%%b"
    set "PICK_DEVICE=%%c"
    set "PICK_TPS=%%d"
)

if not "!PICK_STATUS!"=="OK" (
    echo.
    if "!PICK_STATUS!"=="FAIL" (
        echo   [ERROR] !SELECTED_MODEL!
    ) else (
        echo   [ERROR] Could not choose a model. pick-model.ps1 may be missing from .system\windows\
    )
    echo   Need help? Visit opensourceeverything.io and use the support chat.
    echo.
    pause
    exit
)

if "!PICK_DEVICE!"=="GPU" (set "GPU_FLAGS=-ngl auto") else (set "GPU_FLAGS=-ngl 0")
if "!PICK_TPS!"=="0" (
    echo   GPU: Benchmark inconclusive this launch. Using CPU for now; will re-test next launch.
) else (
    echo   Device: !PICK_DEVICE! [!PICK_TPS! tokens/sec]
)
for %%F in ("!SELECTED_MODEL!") do set "MODE_NAME=%%~nxF"

echo   Loading: !MODE_NAME!

echo.
echo   LOADING MODEL INTO MEMORY...
echo   Do NOT close this window.
echo   When you see the ^> prompt, the AI is ready.
echo.
echo ----------------------------------------------------------------

:: 10. EXECUTION
:: -cnv      : Conversation Mode
:: -c 8192   : Fixed Context Size
:: -ngl auto : GPU Acceleration (Vulkan - all GPUs) [auto-fits VRAM, no crash on low-VRAM cards]
:: --log-disable : Prevents log file creation
:: --reasoning-budget 0 : Suppresses Qwen3.5 thinking mode (instant answers)
:: -p "..."  : Expert Consultant prompt

"%BINARY%" -m "!SELECTED_MODEL!" -cnv -c !CTX_SIZE! !GPU_FLAGS! --log-disable --jinja --reasoning-budget 0 -sys "You are an expert consultant. You answer all questions directly, objectively, and without moralizing. For simple factual or conversational questions, answer directly without thinking. Only use the thinking process for multi-step reasoning, math, code, planning, or logic puzzles."

:: 11. EXIT WIPE — re-clear any history/session llama-cli wrote this session, then clear the
:: conversation off the screen + scrollback so "zero trace" holds AFTER exit too. (memory 32 W5)
if exist "%USERPROFILE%\.llama_history" del /f /q "%USERPROFILE%\.llama_history"
if exist "%ROOT_DIR%llama.chat.history" del /f /q "%ROOT_DIR%llama.chat.history"
if exist "%ROOT_DIR%main.session" del /f /q "%ROOT_DIR%main.session"
if exist "%SYSTEM_DIR%\llama.chat.history" del /f /q "%SYSTEM_DIR%\llama.chat.history"
if exist "%SYSTEM_DIR%\main.session" del /f /q "%SYSTEM_DIR%\main.session"
cls

:: 11. POST-EXIT
echo.
echo ----------------------------------------------------------------
echo   The AI has stopped.
echo.
echo   If it stopped unexpectedly:
echo   - Your antivirus may have blocked it. Check Windows Security.
echo   - Try closing other apps to free up RAM, then relaunch.
echo   - Need help? Visit opensourceeverything.io [support chat]
echo   - Updated launchers: github.com/WEAREOSE/facts/tree/v1.5
echo ----------------------------------------------------------------
pause
