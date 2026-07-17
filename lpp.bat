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
set COMPILED=0

where cl >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    set VCVARS="C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
    if exist !VCVARS! (
        call !VCVARS! > nul
    )
)

where cl >nul 2>nul
if %ERRORLEVEL% EQU 0 (
    echo [L++] Compiling with MSVC cl.exe...
    cl.exe /nologo /O2 output.c /Fe:output.exe /link /SUBSYSTEM:CONSOLE > nul
    if !ERRORLEVEL! EQU 0 (
        set COMPILED=1
    ) else (
        echo [L++] MSVC Compiler failed.
        exit /b !ERRORLEVEL!
    )
) else (
    where gcc >nul 2>nul
    if %ERRORLEVEL% EQU 0 (
        echo [L++] Compiling with GCC...
        gcc output.c -o output.exe
        if !ERRORLEVEL! EQU 0 (
            set COMPILED=1
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
                set COMPILED=1
            ) else (
                echo [L++] C Compiler failed.
                exit /b !ERRORLEVEL!
            )
        ) else (
            echo [L++] No C compiler cl, gcc or clang found in PATH.
            echo [L++] You must compile output.c manually with your preferred C compiler.
        )
    )
)

if !COMPILED! EQU 1 (
    if "!RUN_MODE!"=="1" (
        .\output.exe
        del output.exe > nul 2>nul
        del output.c > nul 2>nul
        del output.obj > nul 2>nul
    ) else (
        echo [L++] Compilation successful! Executable: output.exe
    )
)
