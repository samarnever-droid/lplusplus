<#
.SYNOPSIS
Installs a prebuilt L++ release on Windows. Use `$env:LPP_FROM_SOURCE=1` to build from a local source checkout.
#>

$ErrorActionPreference = "Stop"
$ProjectDir = $PSScriptRoot
$InstallDir = if ($env:LPP_INSTALL_DIR) { $env:LPP_INSTALL_DIR } else { Join-Path $HOME ".lpp" }
$BinDir = Join-Path $InstallDir "bin"
$LibDir = Join-Path $InstallDir "lib"
$Version = if ($env:LPP_VERSION) { $env:LPP_VERSION } else { "v0.1.3" }
$ReleaseUrl = "https://github.com/samarnever-droid/lplusplus/releases/download/$Version/lpp-windows-x86_64.zip"

New-Item -ItemType Directory -Force $BinDir, $LibDir | Out-Null

function Install-Release {
    $temp = Join-Path $env:TEMP "lpp-release-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Force $temp | Out-Null
    try {
        Write-Host "[1/3] Downloading L++ $Version release..." -ForegroundColor Yellow
        Invoke-WebRequest -Uri $ReleaseUrl -OutFile "$temp\lpp.zip" -UseBasicParsing
        Expand-Archive -Path "$temp\lpp.zip" -DestinationPath $temp -Force
        $root = Join-Path $temp "lpp-windows-x86_64"
        if (-not (Test-Path "$root\bin\lpp.exe")) { throw "Release archive is missing lpp.exe" }
        Write-Host "[2/3] Installing compiler, linker, and runtime objects..." -ForegroundColor Yellow
        Copy-Item "$root\bin\lpp.exe" "$BinDir\lpp.exe" -Force
        Copy-Item "$root\bin\lpp-link.exe" "$BinDir\lpp-link.exe" -Force
        Copy-Item "$root\lib\*" $LibDir -Force
        return $true
    } catch {
        Write-Warning "Release installation failed: $($_.Exception.Message)"
        return $false
    } finally {
        Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Install-Source {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "Cargo is required for source installation. Install Rust or use a published release asset."
    }
    Write-Host "[1/3] Building L++ compiler and linker from source..." -ForegroundColor Yellow
    cargo build --release --bin lpp --bin lpp-link
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed." }
    Write-Host "[2/3] Packaging compiler and runtime objects..." -ForegroundColor Yellow
    Copy-Item "$ProjectDir\target\release\lpp.exe" "$BinDir\lpp.exe" -Force
    Copy-Item "$ProjectDir\target\release\lpp-link.exe" "$BinDir\lpp-link.exe" -Force
    Copy-Item "$ProjectDir\lpp_runtime.c" "$LibDir\lpp_runtime.c" -Force
    Copy-Item "$ProjectDir\runtime" "$LibDir\runtime" -Recurse -Force
    $compiled = $false
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vs) {
            cmd.exe /d /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cl.exe /nologo /O2 /c `"$ProjectDir\lpp_runtime.c`" /Fo:`"$LibDir\lpp_runtime.obj`""
            cmd.exe /d /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cl.exe /nologo /O2 /GS- /DLPP_FREESTANDING /c `"$ProjectDir\runtime\windows_x86_64_min.c`" /Fo:`"$LibDir\lpp_runtime_min.obj`""
            $compiled = $true
        }
    }
    if (-not $compiled) {
        if (Get-Command gcc -ErrorAction SilentlyContinue) {
            gcc -O2 -c "$ProjectDir\lpp_runtime.c" -o "$LibDir\lpp_runtime.obj"
            gcc -O2 -fno-stack-protector -DLPP_FREESTANDING -c "$ProjectDir\runtime\windows_x86_64_min.c" -o "$LibDir\lpp_runtime_min.obj"
        } elseif (Get-Command clang -ErrorAction SilentlyContinue) {
            clang -O2 -c "$ProjectDir\lpp_runtime.c" -o "$LibDir\lpp_runtime.obj"
            clang -O2 -fno-stack-protector -DLPP_FREESTANDING -c "$ProjectDir\runtime\windows_x86_64_min.c" -o "$LibDir\lpp_runtime_min.obj"
        }
    }
}

if ($env:LPP_FROM_SOURCE -eq "1") {
    Install-Source
} elseif (-not (Install-Release)) {
    Write-Warning "Falling back to local source installation."
    Install-Source
}

$registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
$currentPath = $registryKey.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if ($currentPath -split ";" -notcontains $BinDir) {
    $registryKey.SetValue("Path", ($currentPath + ";" + $BinDir) -replace ";+", ";", [Microsoft.Win32.RegistryValueKind]::String)
}
$registryKey.Close()
Write-Host "[3/3] Installed commands: lpp, lpp-link" -ForegroundColor Green
Write-Host "Restart your terminal, then run: lpp -h" -ForegroundColor Green
