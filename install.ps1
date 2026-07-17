<#
.SYNOPSIS
    Installs the L++ Compiler and Runtime Library globally on the system,
    adding the 'lpp' command to the PATH.

.EXAMPLE
    .\install.ps1
#>

$ErrorActionPreference = "Stop"
$ProjectDir = $PSScriptRoot
$InstallDir = "$env:USERPROFILE\.lpp"
$BinDir     = "$InstallDir\bin"
$LibDir     = "$InstallDir\lib"

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "             L++ COMPILER GLOBAL INSTALLER              " -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

# 1. Compile compiler in release mode
Write-Host "`n[1/5] Building L++ compiler in release mode..." -ForegroundColor Yellow
$proc = Start-Process -FilePath "cargo" -ArgumentList "build --release" -WorkingDirectory $ProjectDir -NoNewWindow -Wait -PassThru
if ($proc.ExitCode -ne 0) {
    Write-Error "Cargo build failed. Make sure Rust is installed."
    exit 1
}

# 2. Setup install directories
Write-Host "`n[2/5] Setting up global installation directories..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force $BinDir | Out-Null
New-Item -ItemType Directory -Force $LibDir | Out-Null

# 3. Copy binaries and runtime sources
Write-Host "`n[3/5] Installing binaries and runtime libraries..." -ForegroundColor Yellow
Copy-Item -Path "$ProjectDir\target\release\lpp.exe" -Destination "$BinDir\lpp-compiler.exe" -Force
Copy-Item -Path "$ProjectDir\lpp_runtime.c" -Destination "$LibDir\lpp_runtime.c" -Force

# Load MSVC to pre-compile the runtime for global linking
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    $vcvars = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
    if (Test-Path $vcvars) {
        $tempFile = [System.IO.Path]::GetTempFileName()
        cmd.exe /c "call `"$vcvars`" > nul && set > `"$tempFile`""
        Get-Content $tempFile | ForEach-Object {
            if ($_ -match "^([^=]+)=(.*)$") {
                $name = $Matches[1]; $val = $Matches[2]
                Set-Content -Path "env:\$name" -Value $val
            }
        }
        Remove-Item $tempFile
    }
}

if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Write-Host "  Pre-compiling static runtime library object..." -ForegroundColor Yellow
    & cl.exe /nologo /O2 /c "$LibDir\lpp_runtime.c" "/Fo:$LibDir\lpp_runtime.obj" 2>&1 | Out-Null
} else {
    Write-Warning "MSVC compiler (cl.exe) not found. Global linking will fall back to C compilation on link stage."
}

# 4. Generate the global lpp.bat wrapper script
Write-Host "`n[4/5] Generating global 'lpp' CLI command..." -ForegroundColor Yellow
$batContent = @"
@echo off
setlocal enabledelayedexpansion

if "%~1"=="" (
    echo L++ Compiler and Codegen Backend
    echo Usage: lpp [filename.lpp] [options]
    echo.
    echo Options:
    echo   -v, --version    Show L++ compiler version
    echo   -h, --help       Show help menu
    exit /b 1
)

:: Check for simple command line flags
if "%~1"=="--version" (
    "%~dp0lpp-compiler.exe" --version
    exit /b 0
)
if "%~1"=="-v" (
    "%~dp0lpp-compiler.exe" -v
    exit /b 0
)
if "%~1"=="--help" (
    "%~dp0lpp-compiler.exe" --help
    exit /b 0
)
if "%~1"=="-h" (
    "%~dp0lpp-compiler.exe" -h
    exit /b 0
)

:: Ensure file exists
if not exist "%~1" (
    echo [L++] Error: File "%~1" not found.
    exit /b 1
)

:: 1. Compile L++ to object file
set LPP_AOT=1
set BENCHMARK=1
"%~dp0lpp-compiler.exe" "%~1"
set LPP_AOT=
set BENCHMARK=

set OBJ_FILE=%~dpn1.o
if not exist "%OBJ_FILE%" (
    echo [L++] Error: Compilation failed.
    exit /b 1
)

:: 2. Ensure cl.exe / link.exe are on PATH
where cl >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    set VCVARS="C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
    if exist !VCVARS! (
        call !VCVARS! > nul
    ) else (
        echo [L++] Error: MSVC link.exe not found on PATH. Please load vcvars64.bat.
        exit /b 1
    )
)

:: 3. Link object with global runtime
set RUNTIME_OBJ=%~dp0..\lib\lpp_runtime.obj
set RUNTIME_SRC=%~dp0..\lib\lpp_runtime.c

set EXE_FILE=%~dpn1.exe
if "%~2"=="--run" (
    set EXE_FILE=%TEMP%\lpp_temp_%RANDOM%.exe
)

if exist "%RUNTIME_OBJ%" (
    link.exe /nologo "%OBJ_FILE%" "%RUNTIME_OBJ%" /out:"!EXE_FILE!" /SUBSYSTEM:CONSOLE > nul
) else (
    cl.exe /nologo /O2 "%OBJ_FILE%" "%RUNTIME_SRC%" /Fe:"!EXE_FILE!" > nul
)

if %ERRORLEVEL% NEQ 0 (
    echo [L++] Error: Linking native executable failed.
    exit /b 1
)

:: Clean up temporary object file
del "%OBJ_FILE%" > nul 2>nul

if "%~2"=="--run" (
    "!EXE_FILE!"
    del "!EXE_FILE!" > nul 2>nul
) else (
    echo [L++] Compilation successful: "%~dpn1.exe"
)
"@

Set-Content -Path "$BinDir\lpp.bat" -Value $batContent -Encoding Ascii

# 5. Add bin directory to User PATH
Write-Host "`n[5/5] Adding L++ binary directory to PATH..." -ForegroundColor Yellow
$registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
$currentPath = $registryKey.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)

if ($currentPath -split ";" -notcontains $BinDir) {
    $newPath = $currentPath + ";" + $BinDir
    $newPath = $newPath -replace ";+", ";"
    $registryKey.SetValue("Path", $newPath, [Microsoft.Win32.RegistryValueKind]::String)
    Write-Host "  Added $BinDir to Current User PATH." -ForegroundColor Green
    Write-Host "  Please restart your terminal/IDE for the PATH changes to take effect." -ForegroundColor Green
} else {
    Write-Host "  $BinDir is already in your PATH." -ForegroundColor Green
}

$registryKey.Close()

Write-Host "`n========================================================" -ForegroundColor Green
Write-Host "      L++ COMPILER INSTALLED GLOBALLY SUCCESSFULLY!     " -ForegroundColor Green
Write-Host "      Try running: lpp -v                               " -ForegroundColor Green
Write-Host "========================================================" -ForegroundColor Green
