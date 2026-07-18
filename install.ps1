$ErrorActionPreference = "Stop"

$InstallDir = if ($env:LPP_INSTALL_DIR) { $env:LPP_INSTALL_DIR } else { Join-Path $HOME ".lpp" }
$BinDir = Join-Path $InstallDir "bin"
$LibDir = Join-Path $InstallDir "lib"

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "                 L++ DOWNLOAD INSTALLER                 " -ForegroundColor Cyan
Write-Host "========================================================" -ForegroundColor Cyan

Write-Host "`n[1/3] Preparing install directories..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force $BinDir | Out-Null
New-Item -ItemType Directory -Force $LibDir | Out-Null

Write-Host "`n[2/3] Fetching compiler binary and runtime files..." -ForegroundColor Yellow
$baseUrl = "https://github.com/samarnever-droid/lplusplus"
$releaseUrl = "$baseUrl/releases/download/v0.1.0/lpp.exe"
$runtimeUrl = "https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/lpp_runtime.c"

Write-Host "  Downloading compiler from $releaseUrl ..." -ForegroundColor Yellow
Invoke-WebRequest -Uri $releaseUrl -OutFile (Join-Path $BinDir "lpp.exe") -UseBasicParsing

Write-Host "  Downloading runtime source from $runtimeUrl ..." -ForegroundColor Yellow
Invoke-WebRequest -Uri $runtimeUrl -OutFile (Join-Path $LibDir "lpp_runtime.c") -UseBasicParsing

if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Write-Host "  Precompiling runtime object with cl.exe..." -ForegroundColor Yellow
    & cl.exe /nologo /O2 /c (Join-Path $LibDir "lpp_runtime.c") "/Fo:(Join-Path $LibDir lpp_runtime.obj)" 2>&1 | Out-Null
}

Write-Host "`n[3/3] Updating PATH..." -ForegroundColor Yellow
$registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
$currentPath = $registryKey.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)

if ($currentPath -split ";" -notcontains $BinDir) {
    $newPath = ($currentPath + ";" + $BinDir) -replace ";+", ";"
    $registryKey.SetValue("Path", $newPath, [Microsoft.Win32.RegistryValueKind]::String)
    Write-Host "  Added $BinDir to Current User PATH." -ForegroundColor Green
    Write-Host "  Restart your terminal to pick up the PATH change." -ForegroundColor Green
} else {
    Write-Host "  $BinDir is already in PATH." -ForegroundColor Green
}
$registryKey.Close()

Write-Host "`n========================================================" -ForegroundColor Green
Write-Host "         L++ INSTALLED SUCCESSFULLY. TRY: lpp -h        " -ForegroundColor Green
Write-Host "========================================================" -ForegroundColor Green
