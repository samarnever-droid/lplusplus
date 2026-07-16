@echo off
setlocal enabledelayedexpansion

if "%~1"=="" (
    echo Usage: lpp.bat [filename.lpp] [--run]
    exit /b 1
)

set LPP_FILE=%~1
set RUN_MODE=0

if "%~2"=="--run" (
    set RUN_MODE=1
)

echo [L++] Transpiling %LPP_FILE%...
cargo run --manifest-path "%~dp0Cargo.toml" --release %LPP_FILE%
if %ERRORLEVEL% NEQ 0 (
    echo [L++] Transpilation failed.
    exit /b %ERRORLEVEL%
)

echo [L++] Output written to output.c

where gcc >nul 2>nul
if %ERRORLEVEL% EQU 0 (
    echo [L++] Compiling with GCC...
    gcc output.c -o output.exe
    if !ERRORLEVEL! EQU 0 (
        echo [L++] Compilation successful! Executable: output.exe
        if "!RUN_MODE!"=="1" (
            echo [L++] Running output.exe...
            .\output.exe
        )
    ) else (
        echo [L++] C Compiler failed.
        exit /b !ERRORLEVEL!
    )
) else (
    where clang >nul 2>nul
    if !ERRORLEVEL! EQU 0 (
        echo [L++] Compiling with Clang...
        clang output.c -o output.exe
        if !ERRORLEVEL! EQU 0 (
            echo [L++] Compilation successful! Executable: output.exe
            if "!RUN_MODE!"=="1" (
                echo [L++] Running output.exe...
                .\output.exe
            )
        ) else (
            echo [L++] C Compiler failed.
            exit /b !ERRORLEVEL!
        )
    ) else (
        echo [L++] No C compiler gcc or clang found in PATH.
        echo [L++] You must compile output.c manually with your preferred C compiler.
    )
)
