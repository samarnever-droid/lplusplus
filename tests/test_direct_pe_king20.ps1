# King20 direct PE gate. Run only after VSCMD_BAT is supplied by Windows CI.
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Lpp = Join-Path $Root "target\release\lpp.exe"
$Linker = Join-Path $Root "target\release\lpp-link.exe"
$Work = Join-Path $env:RUNNER_TEMP "lpp-king20-direct-pe"
New-Item -ItemType Directory -Force $Work | Out-Null

$cases = @(
    @("benchmarks\bench_fib.lpp", "9227465"),
    @("benchmarks\bench_loop.lpp", "49999995000000"),
    @("benchmarks\bench_calls.lpp", "500000500000"),
    @("tests\arith.lpp", "15`n5`n50`n2"),
    @("tests\branches.lpp", "1`n0`n1"),
    @("tests\nested_calls.lpp", "120"),
    @("tests\closure_test.lpp", "52"),
    @("tests\list_safety.lpp", "3`n5`n13"),
    @("tests\owned_return.lpp", "1"),
    @("tests\arc_branch_return.lpp", "1"),
    @("tests\arc_nested_struct.lpp", "1"),
    @("tests\arc_direct_alias.lpp", "1"),
    @("tests\arc_closure_capture.lpp", "0"),
    @("tests\arc_borrowed_return.lpp", "1"),
    @("tests\arc_borrowed_field_return.lpp", "1"),
    @("tests\arc_field_alias.lpp", "1"),
    @("tests\arc_list_alias.lpp", "7"),
    @("tests\arc_list_custom.lpp", "1"),
    @("tests\arc_nested_branch_alias.lpp", "1"),
    @("tests\arc_closure_branch_capture.lpp", "0")
)

$cmd = Join-Path $Work "build_runtime.cmd"
@"
@echo off
call "%VSCMD_BAT%" >nul || exit /b 1
cl.exe /nologo /O2 /GS- /DLPP_FREESTANDING /c "$Root\runtime\windows_x86_64_min.c" "/Fo:$Work\lpp_runtime_min.obj" || exit /b 1
exit /b 0
"@ | Set-Content -Path $cmd -Encoding ascii
cmd.exe /d /c $cmd
if ($LASTEXITCODE -ne 0) { throw "Failed to compile direct PE runtime" }

$index = 0
foreach ($case in $cases) {
    $index++
    $source = Join-Path $Root $case[0]
    $copied = Join-Path $Work ("{0:D2}.lpp" -f $index)
    Copy-Item $source $copied -Force
    $env:LPP_AOT = "1"
    & $Lpp $copied | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "AOT compile failed for $($case[0])" }
    $exe = Join-Path $Work ("{0:D2}.exe" -f $index)
    & $Linker pe ($copied.Replace(".lpp", ".o")) "$Work\lpp_runtime_min.obj" -o $exe
    if ($LASTEXITCODE -ne 0) { throw "Direct PE link failed for $($case[0])" }
    $actual = (& $exe | ForEach-Object { $_.Trim() }) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $actual -ne $case[1]) {
        throw "King20 PE mismatch for $($case[0]): expected '$($case[1])', got '$actual'"
    }
    Write-Host "PASS direct PE $index/20 $($case[0])"
}

Write-Host "PASS King20 direct PE: 20/20"
