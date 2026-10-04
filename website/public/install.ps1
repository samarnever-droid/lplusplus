<#
.SYNOPSIS
Installs a prebuilt L++ release on Windows. Use `$env:LPP_FROM_SOURCE=1` to build from a local source checkout.
#>

$ErrorActionPreference = "Stop"
$ProjectDir = $PSScriptRoot
$InstallDir = if ($env:LPP_INSTALL_DIR) { $env:LPP_INSTALL_DIR } else { Join-Path $HOME ".lpp" }
$BinDir = Join-Path $InstallDir "bin"
$LibDir = Join-Path $InstallDir "lib"
$Version = if ($env:LPP_VERSION) { $env:LPP_VERSION } else { "latest" }
if (($Version -ne "latest") -and (-not $Version.StartsWith("v"))) { $Version = "v$Version" }
$AssetName = "lpp-windows-x86_64.zip"
if ($Version -eq "latest") {
    $ReleaseBaseUrl = "https://github.com/samarnever-droid/lplusplus/releases/latest/download"
} else {
    $ReleaseBaseUrl = "https://github.com/samarnever-droid/lplusplus/releases/download/$Version"
}
$ReleaseUrl = "$ReleaseBaseUrl/$AssetName"
$ChecksumUrl = "$ReleaseBaseUrl/SHA256SUMS"

New-Item -ItemType Directory -Force $BinDir, $LibDir | Out-Null

function Install-Release {
    $temp = Join-Path $env:TEMP "lpp-release-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Force $temp | Out-Null
    try {
        $archive = Join-Path $temp $AssetName
        $checksumFile = Join-Path $temp "SHA256SUMS"
        Write-Host "[1/3] Downloading L++ $Version release asset and checksum manifest..." -ForegroundColor Yellow
        Invoke-WebRequest -Uri $ReleaseUrl -OutFile $archive -UseBasicParsing
        try {
            Invoke-WebRequest -Uri $ChecksumUrl -OutFile $checksumFile -UseBasicParsing
        } catch {
            throw "Release has no SHA256SUMS manifest; refusing an unverified install."
        }

        $checksumLine = Get-Content $checksumFile | Where-Object { $_ -match "^[0-9A-Fa-f]{64}\s+\*?$([regex]::Escape($AssetName))$" } | Select-Object -First 1
        if (-not $checksumLine) { throw "SHA256SUMS has no valid digest for $AssetName" }
        $expected = ($checksumLine -split "\s+")[0].ToLowerInvariant()
        $actual = (Get-FileHash -Path $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw "SHA-256 verification failed for $AssetName" }

        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [System.IO.Compression.ZipFile]::OpenRead($archive)
        try {
            $destinationRoot = [System.IO.Path]::GetFullPath($temp + [System.IO.Path]::DirectorySeparatorChar)
            $expectedPrefix = "lpp-windows-x86_64/"
            $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
            [long]$expandedBytes = 0
            if ($zip.Entries.Count -gt 10000) { throw "Release archive contains too many entries" }
            foreach ($entry in $zip.Entries) {
                $normalized = $entry.FullName.Replace("\", "/")
                if ((-not $normalized.StartsWith($expectedPrefix, [System.StringComparison]::Ordinal)) -or
                    $normalized.Contains(":") -or (-not $seen.Add($normalized))) {
                    throw "Release archive contains an unsafe or duplicate path: $($entry.FullName)"
                }
                $destination = [System.IO.Path]::GetFullPath((Join-Path $temp $normalized))
                if (-not $destination.StartsWith($destinationRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
                    throw "Release archive path escapes the temporary directory: $($entry.FullName)"
                }
                $expandedBytes += [long]$entry.Length
                if (($entry.Length -gt 536870912) -or ($expandedBytes -gt 1073741824)) {
                    throw "Release archive exceeds the extraction size limit"
                }
                [uint32]$attributes = $entry.ExternalAttributes -band 0xFFFFFFFFL
                $unixType = ($attributes -shr 16) -band 0xF000
                if (($unixType -ne 0) -and ($unixType -ne 0x4000) -and ($unixType -ne 0x8000)) {
                    throw "Release archive contains a link or special file: $($entry.FullName)"
                }
            }
        } finally {
            $zip.Dispose()
        }

        Expand-Archive -Path $archive -DestinationPath $temp -Force
        $root = Join-Path $temp "lpp-windows-x86_64"
        $rootInfo = Get-Item $root -ErrorAction Stop
        $libInfo = Get-Item "$root\lib" -ErrorAction Stop
        $compilerInfo = Get-Item "$root\bin\lpp.exe" -ErrorAction Stop
        $linkerInfo = Get-Item "$root\bin\lpp-link.exe" -ErrorAction Stop
        if ((-not $rootInfo.PSIsContainer) -or ($rootInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Invalid release package root" }
        if ((-not $libInfo.PSIsContainer) -or ($libInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Invalid release library directory" }
        if ($compilerInfo.PSIsContainer -or ($compilerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Release archive is missing a regular lpp.exe" }
        if ($linkerInfo.PSIsContainer -or ($linkerInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Release archive is missing a regular lpp-link.exe" }
        Write-Host "[2/3] Installing verified compiler, linker, and runtime objects..." -ForegroundColor Yellow
        Copy-Item "$root\bin\lpp.exe" "$BinDir\lpp.exe" -Force
        Copy-Item "$root\bin\lpp-link.exe" "$BinDir\lpp-link.exe" -Force
        Copy-Item "$root\lib\*" $LibDir -Recurse -Force
        if (Test-Path "$root\pm") { Remove-Item "$InstallDir\pm" -Recurse -Force -ErrorAction SilentlyContinue; Copy-Item "$root\pm" "$InstallDir\pm" -Recurse -Force }
        if (Test-Path "$root\registry") { Remove-Item "$InstallDir\registry" -Recurse -Force -ErrorAction SilentlyContinue; Copy-Item "$root\registry" "$InstallDir\registry" -Recurse -Force }
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
    Push-Location $ProjectDir
    try {
        cargo build --release --locked --features all-arch --bin lpp --bin lpp-link
        if ($LASTEXITCODE -ne 0) { throw "Cargo build failed." }
    } finally {
        Pop-Location
    }
    Write-Host "[2/3] Packaging compiler and runtime objects..." -ForegroundColor Yellow
    Copy-Item "$ProjectDir\target\release\lpp.exe" "$BinDir\lpp.exe" -Force
    Copy-Item "$ProjectDir\target\release\lpp-link.exe" "$BinDir\lpp-link.exe" -Force
    Copy-Item "$ProjectDir\lpp_runtime.c" "$LibDir\lpp_runtime.c" -Force
    if (Test-Path "$ProjectDir\pm") { Remove-Item "$InstallDir\pm" -Recurse -Force -ErrorAction SilentlyContinue; Copy-Item "$ProjectDir\pm" "$InstallDir\pm" -Recurse -Force }
    if (Test-Path "$ProjectDir\registry") { Remove-Item "$InstallDir\registry" -Recurse -Force -ErrorAction SilentlyContinue; Copy-Item "$ProjectDir\registry" "$InstallDir\registry" -Recurse -Force }
    Copy-Item "$ProjectDir\runtime" "$LibDir\runtime" -Recurse -Force
    $compiled = $false
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vs) {
            cmd.exe /d /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cl.exe /nologo /O2 /c `"$ProjectDir\lpp_runtime.c`" /Fo:`"$LibDir\lpp_runtime.obj`""
            cmd.exe /d /c "call `"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && cl.exe /nologo /O2 /GS- /Gs1000000 /DLPP_FREESTANDING /c `"$ProjectDir\runtime\windows_x86_64_min.c`" /Fo:`"$LibDir\lpp_runtime_min.obj`""
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
    throw "Verified release installation failed. No automatic source fallback was attempted; from a trusted source checkout, set LPP_FROM_SOURCE=1 explicitly."
}

$registryKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
$currentPath = $registryKey.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if ($currentPath -split ";" -notcontains $BinDir) {
    $registryKey.SetValue("Path", ($currentPath + ";" + $BinDir) -replace ";+", ";", [Microsoft.Win32.RegistryValueKind]::String)
}
$registryKey.Close()

# Make lpp available in this PowerShell immediately too.
if (($env:Path -split ";") -notcontains $BinDir) {
    $env:Path = "$BinDir;$env:Path"
}

$InstalledVersion = "unable to execute $BinDir\lpp.exe"
try {
    if (Test-Path "$BinDir\lpp.exe") {
        $InstalledVersion = (& "$BinDir\lpp.exe" -v 2>$null) -join " "
    }
} catch {}

Write-Host "[3/3] Installed commands: lpp, lpp-link" -ForegroundColor Green
Write-Host "Requested release: $Version" -ForegroundColor Green
Write-Host "Release asset: lpp-windows-x86_64.zip" -ForegroundColor Green
Write-Host "Download URL: $ReleaseUrl" -ForegroundColor Green
Write-Host "Installed version: $InstalledVersion" -ForegroundColor Green
Write-Host "Install path: $InstallDir" -ForegroundColor Green
Write-Host "You can run now: lpp -v" -ForegroundColor Green
