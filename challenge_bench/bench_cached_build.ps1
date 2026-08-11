# Tier B: cached end-to-end `lpp build` wall time (cache-hit compile + link).
# This is the number users actually feel on iterative builds.
$ErrorActionPreference = "Stop"
$lpp = "c:\Users\khati\lpp\target\debug\lpp.exe"
$proj = "c:\Users\khati\lpp\my_test_project"
$runs = 21

# Ensure the project is primed (cache populated) with the direct linker.
$env:LPP_LINKER = "direct"
Push-Location $proj
& $lpp build | Out-Null
if ($LASTEXITCODE -ne 0) { throw "priming build failed" }

$times = @()
for ($i = 0; $i -lt $runs; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $lpp build | Out-Null
    $sw.Stop()
    if ($LASTEXITCODE -ne 0) { throw "build failed on run $i" }
    $times += $sw.Elapsed.TotalMilliseconds
}
Pop-Location
Remove-Item Env:\LPP_LINKER

$sorted = $times | Sort-Object
Write-Host "Cached 'lpp build' (direct link), $runs runs:"
Write-Host ("  median = {0:N1} ms" -f $sorted[[int]($runs / 2)])
Write-Host ("  min    = {0:N1} ms" -f $sorted[0])
Write-Host ("  max    = {0:N1} ms" -f $sorted[$runs - 1])
