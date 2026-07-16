<#
.SYNOPSIS
    Compiles and runs L++ source code using the native Cranelift AOT compiler
    and MSVC linker.

.PARAMETER File
    The path to the L++ source file (.lpp) to run.

.EXAMPLE
    .\run.ps1 tests\fib.lpp
#>

param(
    [Parameter(Mandatory=$true)]
    [string]$File
)

$ErrorActionPreference = "Stop"
$ProjectDir = $PSScriptRoot

if (-not (Test-Path $File)) {
    Write-Error "Source file '$File' not found."
    exit 1
}

# 1. Compile L++ code to native x86-64 Object (.o)
Write-Host "[L++] Compiling $File to object file..." -ForegroundColor Cyan
$env:LPP_AOT = "1"
$env:BENCHMARK = "1"

$proc = Start-Process -FilePath "$ProjectDir\target\release\lpp.exe" `
    -ArgumentList "`"$File`"" `
    -WorkingDirectory $ProjectDir `
    -NoNewWindow -Wait -PassThru

$env:LPP_AOT = $null
$env:BENCHMARK = $null

if ($proc.ExitCode -ne 0) {
    Write-Error "L++ compilation failed."
    exit $proc.ExitCode
}

$objFile = $File.Replace(".lpp", ".o")
if (-not (Test-Path $objFile)) {
    Write-Error "Object file was not generated at $objFile"
    exit 1
}

# 2. Check and load MSVC environment (cl.exe and link.exe)
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    Write-Host "[L++] Initializing MSVC Developer Command Prompt..." -ForegroundColor Yellow
    $vcvars = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
    if (-not (Test-Path $vcvars)) {
        Write-Error "Could not find MSVC vcvars64.bat. Please verify Visual Studio installation."
        exit 1
    }
    $tempFile = [System.IO.Path]::GetTempFileName()
    cmd.exe /c "call `"$vcvars`" > nul && set > `"$tempFile`""
    Get-Content $tempFile | ForEach-Object {
        if ($_ -match "^([^=]+)=(.*)$") {
            $name = $Matches[1]
            $val = $Matches[2]
            Set-Content -Path "env:\$name" -Value $val
        }
    }
    Remove-Item $tempFile
}

# 3. Precompile runtime library if not already present
$runtimeSrc = "$ProjectDir\lpp_runtime.c"
$runtimeObj = "$ProjectDir\lpp_runtime.obj"
if (-not (Test-Path $runtimeObj)) {
    Write-Host "[L++] Compiling L++ runtime..." -ForegroundColor Yellow
    & cl.exe /nologo /O2 /c $runtimeSrc "/Fo:$runtimeObj" 2>&1 | Out-Null
}

# 4. Link Object file into native executable
Write-Host "[L++] Linking to native executable..." -ForegroundColor Cyan
$exeFile = $File.Replace(".lpp", ".exe")
& link.exe /nologo $objFile $runtimeObj "/out:$exeFile" /SUBSYSTEM:CONSOLE 2>&1 | Out-Null

# Clean up intermediate object file
Remove-Item $objFile -ErrorAction SilentlyContinue

if (-not (Test-Path $exeFile)) {
    Write-Error "Linking failed."
    exit 1
}

# 5. Execute the compiled native program
Write-Host "[L++] Running compiled program:`n" -ForegroundColor Green
& $exeFile

# Clean up temporary executable after running
Remove-Item $exeFile -ErrorAction SilentlyContinue
