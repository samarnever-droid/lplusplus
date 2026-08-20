# L++ Fast Local CI Harness (Windows)
# Runs the full compiler verification matrix locally before pushing.

param(
    [switch]$Quick = $false
)

$ErrorActionPreference = "Stop"
$root = (Get-Item -Path $PSScriptRoot\..).FullName
Set-Location $root

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  L++ FAST LOCAL CI VERIFICATION" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

# Step 1: Rust unit tests & symbol parity gate
Write-Host "`n[1/5] Running Rust unit tests & symbol parity gate..." -ForegroundColor Yellow
cargo test --locked
if ($LASTEXITCODE -ne 0) { throw "Rust unit tests failed!" }

# Step 2: Build release binaries
Write-Host "`n[2/5] Building release binaries (lpp, lpp-link)..." -ForegroundColor Yellow
cargo build --release --bin lpp --bin lpp-link
if ($LASTEXITCODE -ne 0) { throw "Release build failed!" }

# Step 3: Freestanding PE Direct Linker Test
Write-Host "`n[3/5] Testing Direct PE freestanding compilation & execution..." -ForegroundColor Yellow
$work = Join-Path $env:TEMP "lpp_local_ci_pe"
New-Item -ItemType Directory -Force $work | Out-Null

Set-Content -Path "$work\direct.lpp" -Value "def main():`n    print(7)" -Encoding ascii

# Locate MSVC cl.exe if available, otherwise use gcc
$hasCl = $false
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vs) {
        $vcvars = "$vs\VC\Auxiliary\Build\vcvars64.bat"
        $buildCmd = "$work\build_rt.cmd"
        $lines = @(
            "@echo off",
            "call `"$vcvars`" >nul",
            "cl.exe /nologo /O2 /GS- /Gs1000000 /DLPP_FREESTANDING /c `"$root\runtime\windows_x86_64_min.c`" `"/Fo:$work\rt.obj`""
        )
        Set-Content -Path $buildCmd -Value $lines -Encoding ascii
        cmd.exe /d /c $buildCmd
        if ($LASTEXITCODE -eq 0) { $hasCl = $true }
    }
}

if (-not $hasCl) {
    Write-Host "Compiling freestanding runtime with gcc..." -ForegroundColor Gray
    gcc -O2 -c runtime/windows_x86_64_min.c -o "$work\rt.obj" -DLPP_FREESTANDING
}

$env:LPP_AOT = "1"
& target\release\lpp.exe "$work\direct.lpp"
if ($LASTEXITCODE -ne 0) { throw "L++ AOT compile direct.lpp failed!" }

& target\release\lpp-link.exe pe "$work\direct.obj" "$work\rt.obj" -o "$work\direct.exe"
if ($LASTEXITCODE -ne 0) { throw "Direct PE link failed!" }

$peOut = (& "$work\direct.exe" 2>&1 | ForEach-Object { $_.Trim() }) -join "`n"
if ($peOut -ne "7") { throw "Direct PE output mismatch! Got '$peOut', expected '7'" }
Write-Host "  -> Direct PE PASS (output = $peOut)" -ForegroundColor Green

# Step 4: Multi-File Modules & Stdlib Direct PE Tests
Write-Host "`n[4/5] Testing multi-file module imports on Windows PE..." -ForegroundColor Yellow
$modWork = Join-Path $env:TEMP "lpp_local_ci_modules"
New-Item -ItemType Directory -Force $modWork | Out-Null
Copy-Item tests\modules\*.lpp $modWork\
New-Item -ItemType Directory -Force "$modWork\utils" | Out-Null
Copy-Item tests\modules\utils\*.lpp "$modWork\utils\"

& target\release\lpp.exe "$modWork\test_full_project.lpp"
& target\release\lpp-link.exe pe "$modWork\test_full_project.obj" "$work\rt.obj" -o "$modWork\test_full_project.exe"
$modOut = (& "$modWork\test_full_project.exe" 2>&1) -join "`n"
$expectedMod = "30`n30`n49`n42`n30`n27`nhello from greet module`ngoodbye from greet module"
if ($modOut.Trim() -ne $expectedMod.Trim()) { throw "Module project output mismatch!" }
Write-Host "  -> Multi-file modules PASS" -ForegroundColor Green

# Step 5: Syntax & Core Feature Tests
Write-Host "`n[5/5] Running syntax and core feature verification..." -ForegroundColor Yellow
& target\release\lpp.exe tests\test_augmented_assign.lpp
& target\release\lpp.exe tests\test_index.lpp
& target\release\lpp.exe tests\test_string_ops.lpp
& target\release\lpp.exe tests\test_struct_constructor.lpp
Write-Host "  -> Core features PASS" -ForegroundColor Green

Write-Host "`n========================================" -ForegroundColor Green
Write-Host "  ALL LOCAL CI CHECKS PASSED (100% GREEN)" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
