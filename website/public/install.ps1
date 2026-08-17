<#
.SYNOPSIS
    L++ Compiler & Toolchain Global Installer for Windows
    Hosted on Cloudflare Edge CDN: https://lplusplus.bond/install.ps1
    Usage:
        irm https://lplusplus.bond/install.ps1 | iex
#>

$ErrorActionPreference = "Stop"
$InstallDir = if ($env:LPP_INSTALL_DIR) { $env:LPP_INSTALL_DIR } else { Join-Path $HOME ".lpp" }
$BinDir = Join-Path $InstallDir "bin"
$LibDir = Join-Path $InstallDir "lib"
$Version = if ($env:LPP_VERSION) { $env:LPP_VERSION } else { "v4.7.0" }
if (($Version -ne "latest") -and (-not $Version.StartsWith("v"))) { $Version = "v$Version" }

$ReleaseUrl = "https://github.com/samarnever-droid/lplusplus/releases/download/$Version/lpp-windows-x86_64.zip"
$LatestUrl = "https://github.com/samarnever-droid/lplusplus/releases/latest/download/lpp-windows-x86_64.zip"

Write-Host "========================================================" -ForegroundColor Green
Write-Host "       L++ Compiler & Toolchain Global Installer        " -ForegroundColor Green
Write-Host "========================================================" -ForegroundColor Green

New-Item -ItemType Directory -Force $BinDir, $LibDir | Out-Null

function Try-DownloadRelease {
    $temp = Join-Path $env:TEMP "lpp-install-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Force $temp | Out-Null
    try {
        Write-Host "[1/3] Downloading L++ prebuilt release asset ($Version)..." -ForegroundColor Yellow
        $downloaded = $false
        try {
            Invoke-WebRequest -Uri $ReleaseUrl -OutFile "$temp\lpp.zip" -UseBasicParsing
            $downloaded = $true
        } catch {
            try {
                Invoke-WebRequest -Uri $LatestUrl -OutFile "$temp\lpp.zip" -UseBasicParsing
                $downloaded = $true
            } catch {}
        }
        if (-not $downloaded) { return $false }

        Expand-Archive -Path "$temp\lpp.zip" -DestinationPath $temp -Force
        $root = Join-Path $temp "lpp-windows-x86_64"
        if (-not (Test-Path "$root\bin\lpp.exe")) {
            if (Test-Path "$temp\bin\lpp.exe") {
                $root = $temp
            } else {
                return $false
            }
        }
        Write-Host "[2/3] Installing binary components to $BinDir..." -ForegroundColor Yellow
        Copy-Item "$root\bin\lpp.exe" "$BinDir\lpp.exe" -Force
        if (Test-Path "$root\bin\lpp-link.exe") { Copy-Item "$root\bin\lpp-link.exe" "$BinDir\lpp-link.exe" -Force }
        if (Test-Path "$root\lib") { Copy-Item "$root\lib\*" $LibDir -Recurse -Force -ErrorAction SilentlyContinue }
        if (Test-Path "$root\pm") { Copy-Item "$root\pm" "$InstallDir\pm" -Recurse -Force -ErrorAction SilentlyContinue }
        return $true
    } catch {
        return $false
    } finally {
        Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Try-CargoInstall {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Error "Cargo is required when prebuilt binaries are unavailable. Install Rust from https://rustup.rs"
        exit 1
    }
    Write-Host "[1/3] Compiling L++ toolchain from official repository via Cargo..." -ForegroundColor Yellow
    cargo install --git https://github.com/samarnever-droid/lplusplus --root $InstallDir --force --bin lpp --bin lpp-link
    if ($LASTEXITCODE -ne 0) { throw "Cargo installation failed." }
}

if (Try-DownloadRelease) {
    Write-Host "[3/3] Prebuilt release installation complete." -ForegroundColor Green
} else {
    Write-Host "Prebuilt binary package currently building, compiling from official source..." -ForegroundColor Yellow
    Try-CargoInstall
    Write-Host "[3/3] Source installation complete." -ForegroundColor Green
}

# Update User Environment PATH in Registry
try {
    $registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
    $currentPath = $registryKey.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if ($currentPath -split ";" -notcontains $BinDir) {
        $registryKey.SetValue("Path", ($currentPath + ";" + $BinDir) -replace ";+", ";", [Microsoft.Win32.RegistryValueKind]::String)
    }
    $registryKey.Close()
} catch {}

# Update current process PATH
if (($env:Path -split ";") -notcontains $BinDir) {
    $env:Path = "$BinDir;$env:Path"
}

Write-Host ""
Write-Host "✓ L++ Toolchain installed successfully to: $BinDir\lpp.exe" -ForegroundColor Green
Write-Host "To verify your install, run:" -ForegroundColor White
Write-Host "  lpp --help" -ForegroundColor Cyan
Write-Host "  lpp upgrade --check" -ForegroundColor Cyan
