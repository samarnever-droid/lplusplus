<#
.SYNOPSIS
    L++ Linker — links a compiled L++ object file with the runtime library to produce an executable.

.DESCRIPTION
    Detects available compilers (MSVC cl.exe, clang, gcc) and links:
        <input>.o + lpp_runtime.c  →  <input>.exe

.EXAMPLE
    .\scripts\link.ps1 surprise
    # Produces: surprise.exe
#>

param(
    [Parameter(Mandatory = $true)]
    [string]$Name,           # base name without extension, e.g. "surprise"

    [string]$RuntimeSrc = "$PSScriptRoot\..\lpp_runtime.c",
    [string]$OutDir     = "."
)

$ErrorActionPreference = "Stop"

$ObjFile = "$OutDir\$Name.o"
$ExeFile = "$OutDir\$Name.exe"

if (-not (Test-Path $ObjFile)) {
    Write-Error "Object file not found: $ObjFile"
    exit 1
}
if (-not (Test-Path $RuntimeSrc)) {
    Write-Error "Runtime source not found: $RuntimeSrc"
    exit 1
}

# ── Detect compiler ──────────────────────────────────────────────────────────

function Find-Tool([string[]]$names) {
    foreach ($n in $names) {
        $t = Get-Command $n -ErrorAction SilentlyContinue
        if ($t) { return $t.Source }
    }
    return $null
}

$clang = Find-Tool "clang", "clang-18", "clang-17", "clang-16"
$gcc   = Find-Tool "gcc"
$cl    = Find-Tool "cl"

# ── Link ─────────────────────────────────────────────────────────────────────

$linked = $false

if ($clang) {
    Write-Host "[L++ link] Using clang: $clang"
    & $clang $ObjFile $RuntimeSrc -o $ExeFile -O2 2>&1
    if ($LASTEXITCODE -eq 0) { $linked = $true }
}

if (-not $linked -and $gcc) {
    Write-Host "[L++ link] Using gcc: $gcc"
    & $gcc $ObjFile $RuntimeSrc -o $ExeFile -O2 2>&1
    if ($LASTEXITCODE -eq 0) { $linked = $true }
}

if (-not $linked -and $cl) {
    Write-Host "[L++ link] Using MSVC cl.exe: $cl"
    # MSVC: compile runtime to obj, then link both
    $rtObj = "$OutDir\lpp_runtime_msvc.obj"
    & $cl /nologo /O2 /c $RuntimeSrc /Fo:$rtObj 2>&1
    if ($LASTEXITCODE -eq 0) {
        & $cl /nologo $ObjFile $rtObj /Fe:$ExeFile /link /SUBSYSTEM:CONSOLE 2>&1
        if ($LASTEXITCODE -eq 0) { $linked = $true }
    }
    Remove-Item $rtObj -ErrorAction SilentlyContinue
}

if (-not $linked) {
    Write-Error @"
[L++ link] No C compiler found (tried clang, gcc, cl.exe).
Install any one of them and make sure it is on PATH.
Manual linking:
    clang $ObjFile lpp_runtime.c -o $ExeFile
"@
    exit 1
}

Write-Host "[L++ link] Linked: $ExeFile"
