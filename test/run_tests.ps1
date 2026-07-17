# L++ Regression Test Harness
# Compiles and runs stress_test.lpp and mega_stress_test.lpp, asserting outputs.

$ErrorActionPreference = "Stop"

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "                L++ TEST SUITE RUNNER                   " -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

# Find global lpp wrapper
$lppBat = "C:\Users\khati\.lpp\bin\lpp.bat"
if (-not (Test-Path $lppBat)) {
    Write-Error "Global L++ compiler command wrapper not found at $lppBat. Run .\install.ps1 first."
}

$TestDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$success = $true

# Helper function to run a single test and assert its outputs
function Run-Lpp-Test {
    param(
        [string]$FileName,
        [string[]]$ExpectedOutputs
    )
    
    $SourceFile = Join-Path $TestDir $FileName
    $ExeFile = $SourceFile.Replace(".lpp", ".exe")
    $ObjFile = $SourceFile.Replace(".lpp", ".o")
    
    # Clean old artifacts
    if (Test-Path $ExeFile) { Remove-Item $ExeFile -Force }
    if (Test-Path $ObjFile) { Remove-Item $ObjFile -Force }
    
    Write-Host "`nCompiling $FileName natively..." -ForegroundColor Yellow
    & cmd.exe /c "call `"$lppBat`" `"$SourceFile`""
    
    if (-not (Test-Path $ExeFile)) {
        Write-Error "Failed to generate executable for $FileName"
    }
    
    Write-Host "Running $FileName..." -ForegroundColor Yellow
    $actualLines = & $ExeFile
    
    Write-Host "Verifying assertions..." -ForegroundColor Yellow
    $testPassed = $true
    for ($i = 0; $i -lt $ExpectedOutputs.Length; $i++) {
        $expectedVal = $ExpectedOutputs[$i].Trim()
        $actualVal = $actualLines[$i].Trim()
        
        if ($actualVal -eq $expectedVal) {
            Write-Host "  [PASS] '$expectedVal'" -ForegroundColor Green
        } else {
            Write-Host "  [FAIL] Expected: '$expectedVal' | Actual: '$actualVal'" -ForegroundColor Red
            $testPassed = $false
        }
    }
    
    # Cleanup
    if (Test-Path $ExeFile) { Remove-Item $ExeFile -Force }
    if (Test-Path $ObjFile) { Remove-Item $ObjFile -Force }
    
    return $testPassed
}

# ── TEST 1: General stress test ──────────────────────────────
$expected1 = @(
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

$passed1 = Run-Lpp-Test -FileName "stress_test.lpp" -ExpectedOutputs $expected1
if (-not $passed1) { $success = $false }

# ── TEST 2: Mega stress test ────────────────────────────────
$expectedMega = @(
    "=====================================",
    "        L++ MEGA STRESS TEST         ",
    "=====================================",
    "[TEST 1/4] BST Sorted Insertion...",
    "BST In-Order Traversal (Expected: 20, 30, 40, 50, 60, 70, 80):",
    "20",
    "30",
    "40",
    "50",
    "60",
    "70",
    "80",
    "[TEST 2/4] Ackermann Recursion...",
    "Ackermann(3, 3) (Expected: 61):",
    "61",
    "[TEST 3/4] Collatz Conjecture Steps...",
    "Collatz steps for 27 (Expected: 111):",
    "111",
    "[TEST 4/4] Nested Scoping & Mutations...",
    "Scope outputs (Expected: 14, 3, 2, 1, then returning 1):",
    "14",
    "3",
    "2",
    "1",
    "1",
    "=====================================",
    "        ALL MEGA TESTS PASSED        ",
    "====================================="
)

$passedMega = Run-Lpp-Test -FileName "mega_stress_test.lpp" -ExpectedOutputs $expectedMega
if (-not $passedMega) { $success = $false }

# ── TEST 3: JSON standard library test ───────────────────────
$expectedJson = @(
    "--- JSON Parser Library Test ---",
    "val:",
    "42",
    "name:",
    "L++ Lang",
    "status:",
    "200",
    "--- Test Success ---"
)

$passedJson = Run-Lpp-Test -FileName "json_test.lpp" -ExpectedOutputs $expectedJson
if (-not $passedJson) { $success = $false }

# ── Final Outcome ───────────────────────────────────────────
if ($success) {
    Write-Host "`n========================================================" -ForegroundColor Green
    Write-Host "            ALL REGRESSION TEST SUITES PASSED!          " -ForegroundColor Green
    Write-Host "========================================================" -ForegroundColor Green
} else {
    Write-Host "`n========================================================" -ForegroundColor Red
    Write-Host "                REGRESSION TEST SUITE FAILED!           " -ForegroundColor Red
    Write-Host "========================================================" -ForegroundColor Red
    exit 1
}
