# L++ Stress Test Runner Script
# This script compiles stress_test.lpp, runs it, and asserts the expected output.

$ErrorActionPreference = "Stop"

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "                L++ STRESS TEST RUNNER                  " -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

# Find global lpp wrapper
$lppBat = "C:\Users\khati\.lpp\bin\lpp.bat"
if (-not (Test-Path $lppBat)) {
    Write-Error "Global L++ compiler command wrapper not found at $lppBat. Run .\install.ps1 first."
}

$TestDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$SourceFile = Join-Path $TestDir "stress_test.lpp"
$ExeFile = Join-Path $TestDir "stress_test.exe"
$ObjFile = Join-Path $TestDir "stress_test.o"

# Clean old artifacts
if (Test-Path $ExeFile) { Remove-Item $ExeFile -Force }
if (Test-Path $ObjFile) { Remove-Item $ObjFile -Force }

Write-Host "`n[1/3] Compiling stress_test.lpp natively..." -ForegroundColor Yellow
& cmd.exe /c "call `"$lppBat`" `"$SourceFile`""

if (-not (Test-Path $ExeFile)) {
    Write-Error "Failed to generate executable stress_test.exe"
}
Write-Host "  Compilation successful!" -ForegroundColor Green

Write-Host "`n[2/3] Running stress_test.exe..." -ForegroundColor Yellow
$outputLines = & $ExeFile

# Define expected outputs in order
$expected = @(
    "--- L++ Stress Test ---",
    "Loop Sum (Expected: 4950):",
    "4950",
    "Fibonacci(10) (Expected: 55):",
    "55",
    "Shadowing values (Expected: 30, 20, 10, then return 10):",
    "30",
    "20",
    "10",
    "10",
    "Struct Mutation (Expected: 123, 456, 789, 999):",
    "123",
    "456",
    "789",
    "999",
    "String parsing (Expected: 1337):",
    "1337",
    "--- STRESS TEST PASS ---"
)

Write-Host "`n[3/3] Verifying outputs..." -ForegroundColor Yellow
$success = $true
for ($i = 0; $i -lt $expected.Length; $i++) {
    $expectedVal = $expected[$i]
    $actualVal = $outputLines[$i].Trim()
    
    if ($actualVal -eq $expectedVal) {
        Write-Host "  [PASS] Expected: '$expectedVal' | Actual: '$actualVal'" -ForegroundColor Green
    } else {
        Write-Host "  [FAIL] Expected: '$expectedVal' | Actual: '$actualVal'" -ForegroundColor Red
        $success = $false
    }
}

# Cleanup
if (Test-Path $ExeFile) { Remove-Item $ExeFile -Force }
if (Test-Path $ObjFile) { Remove-Item $ObjFile -Force }

if ($success) {
    Write-Host "`n========================================================" -ForegroundColor Green
    Write-Host "            ALL STRESS TEST ASSERTIONS PASSED!          " -ForegroundColor Green
    Write-Host "========================================================" -ForegroundColor Green
} else {
    Write-Host "`n========================================================" -ForegroundColor Red
    Write-Host "                STRESS TEST VERIFICATION FAILED!        " -ForegroundColor Red
    Write-Host "========================================================" -ForegroundColor Red
    exit 1
}
