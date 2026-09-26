# Build the complete Aura / EnvBox NSIS installer.
# This command always rebuilds the Rust workspace and both Runtime DLLs.
[CmdletBinding()]
param(
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version = "0.3.0",
    [string]$Nsis = ""
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$artifacts = Join-Path $root "artifacts"
$rel = Join-Path $root "target\release"
New-Item -ItemType Directory -Force $artifacts | Out-Null

function Find-CMake {
    $candidates = @(
        "D:\Tools\VS2022BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $command = Get-Command cmake.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    return $null
}

function Build-Runtime {
    param(
        [string]$Arch,
        [string]$CMakeArch,
        [string]$DllName,
        [string]$BuildDirName
    )

    $cmake = Find-CMake
    if (-not $cmake) { throw "cmake not found. Install VS Build Tools CMake or put cmake on PATH." }

    $detours = $env:DETOURS_ROOT
    if (-not $detours) { $detours = "D:\Tools\Detours" }
    if (-not (Test-Path -LiteralPath $detours -PathType Container)) {
        throw "Detours not found at $detours. Set DETOURS_ROOT or build Detours first."
    }

    $buildDir = Join-Path $root "target\$BuildDirName"
    Write-Host "== runtime $Arch cmake (Detours: $detours) =="
    & $cmake -S (Join-Path $root "runtime") -B $buildDir -G "Visual Studio 17 2022" -A $CMakeArch "-DDETOURS_ROOT=$detours"
    if ($LASTEXITCODE -ne 0) { throw "cmake configure failed for $Arch ($LASTEXITCODE)" }
    # Packaging requires a freshly compiled and linked Runtime DLL on every run.
    & $cmake --build $buildDir --config Release --target "envbox-$DllName" --clean-first
    if ($LASTEXITCODE -ne 0) { throw "cmake build failed for $Arch ($LASTEXITCODE)" }

    $dll = Join-Path $buildDir "Release\envbox-$DllName.dll"
    if (-not (Test-Path -LiteralPath $dll -PathType Leaf)) {
        throw "runtime build succeeded but output is missing: $dll"
    }
    New-Item -ItemType Directory -Force $rel | Out-Null
    Copy-Item -LiteralPath $dll -Destination (Join-Path $rel "envbox-$DllName.dll") -Force
}

$setup = Join-Path $artifacts "aura-setup.exe"
if (Test-Path -LiteralPath $setup -PathType Leaf) {
    Remove-Item -LiteralPath $setup -Force
}

$manifest = Join-Path $root "Cargo.toml"
Write-Host "== cargo clean --workspace --release =="
& cargo clean --workspace --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw "cargo release clean failed ($LASTEXITCODE)" }

Write-Host "== cargo build --workspace --release =="
& cargo build --workspace --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw "cargo release build failed ($LASTEXITCODE)" }

Build-Runtime -Arch "x64" -CMakeArch "x64" -DllName "runtime64" -BuildDirName "runtime-build"
Build-Runtime -Arch "x86" -CMakeArch "Win32" -DllName "runtime32" -BuildDirName "runtime-build32"

# Stage every file expected by the NSIS script from this release build.
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
    $src = Join-Path $rel $name
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) {
        throw "missing release binary: $src"
    }
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
& $Nsis "/DVERSION=$Version" $nsi
if ($LASTEXITCODE -ne 0) { throw "makensis failed ($LASTEXITCODE)" }

if (-not (Test-Path -LiteralPath $setup)) { throw "expected setup missing: $setup" }
$size = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host ""
Write-Host "Installer: $setup (${size} MB)  version=$Version"
Write-Host "Silent:    `"$setup`" /S"
