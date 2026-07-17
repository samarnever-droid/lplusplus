<#
.SYNOPSIS
    Compiles and runs a single L++ source file.

.DESCRIPTION
    Uses the release compiler to emit a native object file and then links it
    with either MSVC, MinGW GCC, or Clang, depending on what is available.

.EXAMPLE
    .\run.ps1 tests\fib.lpp
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$File
)

$ErrorActionPreference = "Stop"
$ProjectDir = $PSScriptRoot
$Compiler = Join-Path $ProjectDir "target\release\lpp.exe"
$RuntimeSource = Join-Path $ProjectDir "lpp_runtime.c"
$RuntimeObject = Join-Path $ProjectDir "lpp_runtime.obj"

if (-not (Test-Path $File)) {
    Write-Error "Source file '$File' not found."
    exit 1
}

if (-not (Test-Path $Compiler)) {
    Write-Host "[L++] Building release compiler..." -ForegroundColor Yellow
    cargo build --release | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Failed to build target\release\lpp.exe"
        exit 1
    }
}

Write-Host "[L++] Compiling $File to object file..." -ForegroundColor Cyan
$env:LPP_AOT = "1"
$env:BENCHMARK = "1"
& $Compiler $File
$compileExit = $LASTEXITCODE
$env:LPP_AOT = $null
$env:BENCHMARK = $null

if ($compileExit -ne 0) {
    Write-Error "L++ compilation failed."
    exit $compileExit
}

$sourcePath = [System.IO.Path]::GetFullPath($File)
$objFile = [System.IO.Path]::ChangeExtension($sourcePath, ".o")
$exeFile = [System.IO.Path]::ChangeExtension($sourcePath, ".exe")
if (-not (Test-Path $objFile)) {
    Write-Error "Object file was not generated at $objFile"
    exit 1
}

$linked = $false
if (Get-Command link.exe -ErrorAction SilentlyContinue) {
    if (-not (Test-Path $RuntimeObject) -and (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
        & cl.exe /nologo /O2 /c $RuntimeSource "/Fo:$RuntimeObject" 2>&1 | Out-Null
    }
    if (Test-Path $RuntimeObject) {
        Write-Host "[L++] Linking with link.exe..." -ForegroundColor Cyan
        & link.exe /nologo $objFile $RuntimeObject "/out:$exeFile" /SUBSYSTEM:CONSOLE 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $linked = $true
        }
    }
}

if (-not $linked -and (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    Write-Host "[L++] Linking with cl.exe..." -ForegroundColor Cyan
    & cl.exe /nologo /O2 $objFile $RuntimeSource "/Fe:$exeFile" 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        $linked = $true
    }
}

if (-not $linked -and (Get-Command gcc -ErrorAction SilentlyContinue)) {
    Write-Host "[L++] Linking with gcc..." -ForegroundColor Cyan
    & gcc $objFile $RuntimeSource -O2 -o $exeFile
    if ($LASTEXITCODE -eq 0) {
        $linked = $true
    }
}

if (-not $linked -and (Get-Command clang -ErrorAction SilentlyContinue)) {
    Write-Host "[L++] Linking with clang..." -ForegroundColor Cyan
    & clang $objFile $RuntimeSource -O2 -o $exeFile
    if ($LASTEXITCODE -eq 0) {
        $linked = $true
    }
}

Remove-Item $objFile -ErrorAction SilentlyContinue

if (-not $linked -or -not (Test-Path $exeFile)) {
    Write-Error "Linking failed. Install MSVC Build Tools, GCC, or Clang."
    exit 1
}

Write-Host "[L++] Running compiled program:`n" -ForegroundColor Green
& $exeFile
$runExit = $LASTEXITCODE
Remove-Item $exeFile -ErrorAction SilentlyContinue
exit $runExit
