$ErrorActionPreference = "Stop"

# Publish all monorepo packages to llppregistry in topological order.
# Requires git push credentials for https://github.com/samarnever-droid/llppregistry.git.

$KeelBin = if ($env:KEEL_BIN) { $env:KEEL_BIN } else { "keel" }
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

$LeafPackages = @(
    "lpp-semver",
    "lpp-toml",
    "lpp-math",
    "lpp-graph",
    "lpp-zip",
    "lpp-zundo",
    "lpp-bindgen",
    "simple-markdown",
    "simple-pdf",
    "compresslpp",
    "lpp-analyzer",
    "lpp-engine",
    "lpp-git",
    "lppdb",
    "lppsqlite",
    "lppstore",
    "lreact"
)

foreach ($pkg in $LeafPackages) {
    $dir = Join-Path $RepoRoot "packages\$pkg"
    if (Test-Path (Join-Path $dir "Keel.toml")) {
        Write-Host "==> Publishing $pkg..." -ForegroundColor Cyan
        Push-Location $dir
        try {
            & $KeelBin publish
            if ($LASTEXITCODE -ne 0) {
                Write-Warning "Skipped or already published: $pkg"
            }
        } finally {
            Pop-Location
        }
    }
}
