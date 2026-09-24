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

$build = Join-Path $root "target/runtime-build"
Write-Host "== runtime cmake =="
& $cmake -S (Join-Path $root "runtime") -B $build -G "Visual Studio 17 2022" -A x64
& $cmake --build $build --config Release
& $cmake -S (Join-Path $root "runtime") -B (Join-Path $root "target/runtime-build-x86") -G "Visual Studio 17 2022" -A Win32
& $cmake --build (Join-Path $root "target/runtime-build-x86") --config Release

Write-Host "Done."
