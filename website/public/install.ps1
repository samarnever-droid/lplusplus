<#
.SYNOPSIS
    Installs the L++ compiler and runtime assets to a per-user directory.

.DESCRIPTION
    Builds the release compiler, copies `lpp.exe` and `lpp_runtime.c` into
    `$HOME\.lpp`, and optionally precompiles `lpp_runtime.obj` when MSVC is
    available. The installed `lpp.exe` is the primary CLI entrypoint.

.EXAMPLE
    .\install.ps1
#>

$ErrorActionPreference = "Stop"

$ProjectDir = $PSScriptRoot
$InstallDir = if ($env:LPP_INSTALL_DIR) { $env:LPP_INSTALL_DIR } else { Join-Path $HOME ".lpp" }
$BinDir = Join-Path $InstallDir "bin"
$LibDir = Join-Path $InstallDir "lib"
$CompilerSource = Join-Path $ProjectDir "target\release\lpp.exe"
$CompilerDest = Join-Path $BinDir "lpp.exe"
$RuntimeSource = Join-Path $ProjectDir "lpp_runtime.c"
$RuntimeObject = Join-Path $LibDir "lpp_runtime.obj"

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "                 L++ GLOBAL INSTALLER                   " -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

Write-Host "`n[1/4] Building release compiler and linker MVP..." -ForegroundColor Yellow
$proc = Start-Process -FilePath "cargo" -ArgumentList "build --release --bin lpp --bin lpp-link" -WorkingDirectory $ProjectDir -NoNewWindow -Wait -PassThru
if ($proc.ExitCode -ne 0) {
    Write-Error "Cargo build failed. Make sure Rust is installed and cargo is on PATH."
    exit 1
}

Write-Host "`n[2/4] Preparing install directories..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force $BinDir | Out-Null
New-Item -ItemType Directory -Force $LibDir | Out-Null

Write-Host "`n[3/4] Installing compiler and runtime files..." -ForegroundColor Yellow
Copy-Item -Path $CompilerSource -Destination $CompilerDest -Force
$LinkerSource = Join-Path $ProjectDir "target\release\lpp-link.exe"
if (Test-Path $LinkerSource) {
    Copy-Item -Path $LinkerSource -Destination (Join-Path $BinDir "lpp-link.exe") -Force
}
Copy-Item -Path $RuntimeSource -Destination (Join-Path $LibDir "lpp_runtime.c") -Force

if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Write-Host "  Precompiling runtime object with cl.exe..." -ForegroundColor Yellow
    & cl.exe /nologo /O2 /c (Join-Path $LibDir "lpp_runtime.c") "/Fo:$RuntimeObject" 2>&1 | Out-Null
} else {
    Write-Host "  MSVC not detected. Native builds will compile lpp_runtime.c at link time." -ForegroundColor DarkYellow
}

Write-Host "`n[4/4] Updating PATH guidance..." -ForegroundColor Yellow
$registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
$currentPath = $registryKey.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)

if ($currentPath -split ";" -notcontains $BinDir) {
    $newPath = ($currentPath + ";" + $BinDir) -replace ";+", ";"
    $registryKey.SetValue("Path", $newPath, [Microsoft.Win32.RegistryValueKind]::String)
    Write-Host "  Added $BinDir to Current User PATH." -ForegroundColor Green
    Write-Host "  Restart your terminal to pick up the PATH change." -ForegroundColor Green
} else {
    Write-Host "  $BinDir is already present in Current User PATH." -ForegroundColor Green
}
$registryKey.Close()

Write-Host "`n========================================================" -ForegroundColor Green
Write-Host "       L++ INSTALLED. TRY: lpp.exe -h OR lpp -h         " -ForegroundColor Green
Write-Host "========================================================" -ForegroundColor Green
