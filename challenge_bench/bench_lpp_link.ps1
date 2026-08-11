# lpp-link speed benchmark: 21 link runs per workload, report median/min/max.
$ErrorActionPreference = "Stop"
$dir   = Split-Path -Parent $MyInvocation.MyCommand.Path
$root  = Split-Path -Parent $dir
$link  = Join-Path $root "target\release\lpp-link.exe"
$rt    = Join-Path $env:USERPROFILE ".lpp\lib\lpp_runtime_min.obj"
$runs  = 21

$workloads = @("hello_world", "fibonacci", "network_echo_client", "big_gen")
$results = @()

foreach ($w in $workloads) {
    $obj = Join-Path $dir "$w.obj"
    $out = Join-Path $dir "$w.bench.exe"
    $times = @()
    for ($i = 0; $i -lt $runs; $i++) {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $link pe $obj $rt -o $out | Out-Null
        $sw.Stop()
        if ($LASTEXITCODE -ne 0) { throw "lpp-link failed for $w" }
        $times += $sw.Elapsed.TotalMilliseconds
    }
    $sorted = $times | Sort-Object
    $median = $sorted[[int]($runs / 2)]
    $min = $sorted[0]
    $max = $sorted[$runs - 1]
    $avg = ($times | Measure-Object -Average).Average
    $results += [pscustomobject]@{
        Workload = $w
        MedianMs = [math]::Round($median, 3)
        MinMs    = [math]::Round($min, 3)
        MaxMs    = [math]::Round($max, 3)
        AvgMs    = [math]::Round($avg, 3)
    }
}

# Verify the produced binaries actually run
foreach ($w in @("hello_world", "fibonacci")) {
    $exe = Join-Path $dir "$w.bench.exe"
    & $exe | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "$w.bench.exe exited with $LASTEXITCODE" }
}

$results | Format-Table -AutoSize
Write-Host "Machine: $((Get-CimInstance Win32_Processor).Name)"
Write-Host "lpp-link size: $((Get-Item $link).Length) bytes"
