# Build Kite-style NSIS installer for Aura / EnvBox.
# Usage:
#   .\scripts\build-installer.ps1              # release build + stage + makensis
#   .\scripts\build-installer.ps1 -SkipBuild   # reuse existing binaries
param(
    [string]$Version = "0.3.0",
    [switch]$SkipBuild,
    [string]$Nsis = ""
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force $artifacts | Out-Null

if (-not $SkipBuild) {
    Write-Host "== cargo build --release =="
    & cargo build --release --manifest-path (Join-Path $root "Cargo.toml")
    if ($LASTEXITCODE -ne 0) { throw "cargo release build failed" }

    # Runtime DLLs (CMake/MSVC). build.ps1 also rebuilds the workspace — SkipBuild
    # callers already have binaries staged from a prior run.
    $buildPs1 = Join-Path $PSScriptRoot "build.ps1"
    if (Test-Path -LiteralPath $buildPs1) {
        Write-Host "== scripts/build.ps1 -Release -X86 (runtime DLLs) =="
        & $buildPs1 -Release -X86
        if ($LASTEXITCODE -ne 0) { Write-Warning "build.ps1 exited $LASTEXITCODE (continuing with existing DLLs)" }
    }
}

# Stage every binary the .nsi File list expects, preferring release then artifacts.
$rel = Join-Path $root "target\release"
$bins = @(
    "envbox-app.exe",
    "envbox.exe",
    "envbox-broker.exe",
    "envbox-probe.exe",
    "envbox-browser-probe.exe",
    "envbox-suspended-helper.exe",
    "envbox-runtime64.dll",
    "envbox-runtime32.dll"
)
foreach ($name in $bins) {
    $src = $null
    foreach ($cand in @((Join-Path $rel $name), (Join-Path $artifacts $name), (Join-Path (Join-Path $root "target\debug") $name))) {
        if (Test-Path -LiteralPath $cand) { $src = $cand; break }
    }
    if (-not $src) { throw "missing binary for installer: $name" }
    Copy-Item -LiteralPath $src -Destination (Join-Path $artifacts $name) -Force
    Write-Host "  staged $name"
}

if (-not $Nsis) {
    $Nsis = @(
        (Get-Command makensis.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
        "C:\Program Files (x86)\NSIS\makensis.exe",
        "C:\Program Files\NSIS\makensis.exe"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Nsis) { throw "NSIS not found. Install with: winget install NSIS.NSIS" }

$nsi = Join-Path $root "installer\aura.nsi"
Write-Host "== makensis $nsi =="
& $Nsis $nsi
if ($LASTEXITCODE -ne 0) { throw "makensis failed ($LASTEXITCODE)" }

$setup = Join-Path $artifacts "aura-setup.exe"
if (-not (Test-Path -LiteralPath $setup)) { throw "expected setup missing: $setup" }
$size = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host ""
Write-Host "Installer: $setup (${size} MB)  version=$Version"
Write-Host "Silent:    `"$setup`" /S"
