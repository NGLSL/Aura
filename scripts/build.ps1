# EnvBox build helper (PowerShell)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
if (-not $root) { $root = (Get-Location).Path }

Write-Host "== cargo build / test =="
Push-Location $root
cargo build --workspace
cargo test --workspace
Pop-Location

$cmake = "D:\Tools\VS2022BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
if (-not (Test-Path $cmake)) {
    $cmake = "cmake"
}

$detours = $env:DETOURS_ROOT
if (-not $detours) { $detours = "D:\Tools\Detours" }

$build = Join-Path $root "target/runtime-build"
Write-Host "== runtime cmake (Detours: $detours) =="
& $cmake -S (Join-Path $root "runtime") -B $build -G "Visual Studio 17 2022" -A x64 "-DDETOURS_ROOT=$detours"
& $cmake --build $build --config Release

$dll = Join-Path $build "Release\envbox-runtime64.dll"
if (-not (Test-Path $dll)) {
    # multi-config vs single-config layouts
    $alt = Join-Path $build "envbox-runtime64.dll"
    if (Test-Path $alt) { $dll = $alt }
}
$dest = Join-Path $root "target\debug"
if (-not (Test-Path $dest)) { New-Item -ItemType Directory -Path $dest | Out-Null }
if (Test-Path $dll) {
    Copy-Item $dll (Join-Path $dest "envbox-runtime64.dll") -Force
    Write-Host "Copied $dll -> $dest"
} else {
    Write-Warning "envbox-runtime64.dll not found after build"
}

Write-Host "Done."
