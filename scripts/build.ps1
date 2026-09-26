# EnvBox one-shot build (Kite-style).
# Usage:
#   .\scripts\build.ps1              # debug workspace + runtime64 + copy next to exes
#   .\scripts\build.ps1 -Release     # release build
#   .\scripts\build.ps1 -Run         # after build, launch envbox-app.exe
#   .\scripts\build.ps1 -SkipRuntime # Rust only
#   .\scripts\build.ps1 -X86         # also build envbox-runtime32.dll
param(
    [switch]$Release,
    [switch]$Run,
    [switch]$SkipRuntime,
    [switch]$X86
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null

$profile = if ($Release) { "release" } else { "debug" }
$cargoArgs = @("build", "--workspace")
if ($Release) { $cargoArgs += "--release" }

Write-Host "== cargo $($cargoArgs -join ' ') =="
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }

function Find-CMake {
    $candidates = @(
        "D:\Tools\VS2022BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    )
    foreach ($c in $candidates) {
        if (Test-Path -LiteralPath $c) { return $c }
    }
    $cmd = Get-Command cmake.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

function Build-Runtime {
    param([string]$Arch, [string]$CMakeArch, [string]$DllName, [string]$BuildDirName)

    $cmake = Find-CMake
    if (-not $cmake) { throw "cmake not found. Install VS Build Tools CMake or put cmake on PATH." }

    $detours = $env:DETOURS_ROOT
    if (-not $detours) { $detours = "D:\Tools\Detours" }
    if (-not (Test-Path -LiteralPath $detours)) {
        throw "Detours not found at $detours. Set DETOURS_ROOT or build Detours first."
    }

    $build = Join-Path $root "target/$BuildDirName"
    Write-Host "== runtime $Arch cmake (Detours: $detours) =="
    & $cmake -S (Join-Path $root "runtime") -B $build -G "Visual Studio 17 2022" -A $CMakeArch "-DDETOURS_ROOT=$detours"
    if ($LASTEXITCODE -ne 0) { throw "cmake configure failed for $Arch" }
    & $cmake --build $build --config Release --target envbox-$DllName
    if ($LASTEXITCODE -ne 0) { throw "cmake build failed for $Arch" }

    $dll = Join-Path $build "Release\envbox-$DllName.dll"
    if (-not (Test-Path -LiteralPath $dll)) {
        $alt = Join-Path $build "envbox-$DllName.dll"
        if (Test-Path -LiteralPath $alt) { $dll = $alt } else { throw "envbox-$DllName.dll not found after build" }
    }

    # Launcher resolves DLL next to the binary (and in deps for cargo test).
    foreach ($dest in @(
            (Join-Path $root "target\$profile"),
            (Join-Path $root "target\$profile\deps"),
            (Join-Path $root "target\debug"),
            (Join-Path $root "target\debug\deps"),
            $artifacts
        )) {
        if (-not (Test-Path -LiteralPath $dest)) {
            New-Item -ItemType Directory -Force -Path $dest | Out-Null
        }
        Copy-Item -LiteralPath $dll -Destination (Join-Path $dest "envbox-$DllName.dll") -Force
    }
    Write-Host "Copied envbox-$DllName.dll -> target\$profile, deps, artifacts"
}

if (-not $SkipRuntime) {
    Build-Runtime -Arch "x64" -CMakeArch "x64" -DllName "runtime64" -BuildDirName "runtime-build"
    if ($X86) {
        Build-Runtime -Arch "x86" -CMakeArch "Win32" -DllName "runtime32" -BuildDirName "runtime-build32"
    }
}

# Publish GUI + CLI + probes into artifacts/ (GUI "启动环境探针" looks next to itself)
foreach ($name in @("envbox-app.exe", "envbox.exe", "envbox-probe.exe", "envbox-browser-probe.exe")) {
    $src = Join-Path $root "target\$profile\$name"
    if (Test-Path -LiteralPath $src) {
        Copy-Item -LiteralPath $src -Destination (Join-Path $artifacts $name) -Force
        Write-Host "Created: $(Join-Path $artifacts $name)"
    }
}

$gui = Join-Path $artifacts "envbox-app.exe"
Write-Host ""
Write-Host "Done. Launch GUI:"
Write-Host "  $gui"
Write-Host "Or:  .\target\$profile\envbox-app.exe"

if ($Run) {
    if (-not (Test-Path -LiteralPath $gui)) { throw "envbox-app.exe missing" }
    Write-Host "Starting envbox-app..."
    Start-Process -FilePath $gui -WorkingDirectory (Split-Path -Parent $gui)
}
