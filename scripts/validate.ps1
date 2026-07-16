<#
.SYNOPSIS
    L++ Runtime Validation Suite

.DESCRIPTION
    Compiles each test .lpp file through the L++ compiler + C transpiler path,
    links with the runtime, runs the binary, and checks the output against the
    expected value.  Results are reported as PASS / FAIL with a summary table.

.EXAMPLE
    .\scripts\validate.ps1
#>

$ErrorActionPreference = "Stop"

# ── Configuration ─────────────────────────────────────────────────────────────

$ScriptDir  = $PSScriptRoot
$ProjectDir = Split-Path $ScriptDir -Parent
$TestDir    = "$ProjectDir\tests"
$LppBat     = "$ProjectDir\lpp.bat"
$RuntimeSrc = "$ProjectDir\lpp_runtime.c"
$TempDir    = "$env:TEMP\lpp_validate"

New-Item -ItemType Directory -Force $TempDir | Out-Null

# Each entry: @(source_file, expected_output_lines_as_string)
$Tests = @(
    @("arith.lpp",        "15`n5`n50`n2"),
    @("loop.lpp",         "5050"),
    @("branches.lpp",     "1`n0`n1"),
    @("nested_calls.lpp", "120"),
    @("fib.lpp",          "55")
)

# ── Find a C compiler ─────────────────────────────────────────────────────────

function Find-Cc {
    foreach ($n in "clang","gcc","cl") {
        $t = Get-Command $n -ErrorAction SilentlyContinue
        if ($t) { return $n }
    }
    return $null
}

$CC = Find-Cc
if (-not $CC) {
    Write-Warning "No C compiler found — will test compiler output only (no execution)."
}

# ── Helpers ───────────────────────────────────────────────────────────────────

function Run-Lpp([string]$src) {
    $base = [System.IO.Path]::GetFileNameWithoutExtension($src)
    # Run lpp compiler (C-transpiler mode) producing output.c in TempDir
    Push-Location $TempDir
    Copy-Item $src "$TempDir\input.lpp" -Force
    $out = & cmd /c "cd /d `"$ProjectDir`" && lpp.bat `"$TempDir\input.lpp`"" 2>&1
    Pop-Location
    return $LASTEXITCODE
}

function Compile-And-Run([string]$srcFile) {
    $base   = [System.IO.Path]::GetFileNameWithoutExtension($srcFile)
    $cOut   = "$TempDir\output.c"
    $exeOut = "$TempDir\$base.exe"

    # Step 1: L++ → C
    Push-Location $TempDir
    Copy-Item $srcFile "$TempDir\input.lpp" -Force

    # We run lpp from project dir so paths resolve
    $proc = Start-Process -FilePath "cmd.exe" `
        -ArgumentList "/c `"`"$LppBat`" `"$TempDir\input.lpp`"`"" `
        -WorkingDirectory $ProjectDir `
        -RedirectStandardOutput "$TempDir\lpp_stdout.txt" `
        -RedirectStandardError  "$TempDir\lpp_stderr.txt" `
        -NoNewWindow -Wait -PassThru

    Pop-Location

    if ($proc.ExitCode -ne 0) {
        $err = Get-Content "$TempDir\lpp_stderr.txt" -Raw 2>$null
        return @{ ok=$false; err="L++ compile failed: $err" }
    }

    # Copy the generated output.c from project dir
    $generatedC = "$ProjectDir\output.c"
    if (-not (Test-Path $generatedC)) {
        return @{ ok=$false; err="No output.c generated" }
    }
    Copy-Item $generatedC $cOut -Force

    if (-not $CC) {
        return @{ ok=$false; err="No C compiler to run binary" }
    }

    # Step 2: C + runtime → exe
    if ($CC -eq "cl") {
        $rtObj = "$TempDir\rt.obj"
        & cl /nologo /O2 /c $RuntimeSrc /Fo:$rtObj | Out-Null
        & cl /nologo $cOut $rtObj /Fe:$exeOut /link /SUBSYSTEM:CONSOLE 2>&1 | Out-Null
        Remove-Item $rtObj -ErrorAction SilentlyContinue
    } else {
        & $CC $cOut $RuntimeSrc -o $exeOut -O2 2>&1 | Out-Null
    }

    if (-not (Test-Path $exeOut)) {
        return @{ ok=$false; err="Linking failed" }
    }

    # Step 3: Run
    $result = & $exeOut 2>&1
    return @{ ok=$true; output=($result -join "`n") }
}

# ── Run Tests ─────────────────────────────────────────────────────────────────

$pass = 0; $fail = 0
$rows = @()

foreach ($t in $Tests) {
    $srcName, $expected = $t
    $srcFile = "$TestDir\$srcName"

    if (-not (Test-Path $srcFile)) {
        $rows += [PSCustomObject]@{ Test=$srcName; Result="SKIP"; Notes="File not found" }
        continue
    }

    Write-Host -NoNewline "  Testing $srcName ... "

    $r = Compile-And-Run $srcFile

    if (-not $r.ok) {
        Write-Host "FAIL [$($r.err)]" -ForegroundColor Red
        $rows += [PSCustomObject]@{ Test=$srcName; Result="FAIL"; Notes=$r.err }
        $fail++
        continue
    }

    $got  = $r.output.Trim()
    $want = $expected.Trim()

    if ($got -eq $want) {
        Write-Host "PASS" -ForegroundColor Green
        $rows += [PSCustomObject]@{ Test=$srcName; Result="PASS"; Notes="" }
        $pass++
    } else {
        Write-Host "FAIL (output mismatch)" -ForegroundColor Red
        Write-Host "    Expected: $want"
        Write-Host "    Got:      $got"
        $rows += [PSCustomObject]@{ Test=$srcName; Result="FAIL"; Notes="want='$want' got='$got'" }
        $fail++
    }
}

# ── Summary ───────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "─────────────────────────────────────"
$rows | Format-Table -AutoSize
Write-Host "  PASSED: $pass   FAILED: $fail   TOTAL: $($pass + $fail)"
Write-Host "─────────────────────────────────────"

if ($fail -gt 0) { exit 1 }
