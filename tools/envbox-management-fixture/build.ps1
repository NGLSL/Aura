param(
    [ValidateSet('x64', 'x86')]
    [string]$Architecture = 'x64'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$build = Join-Path $repo "target/envbox-management-fixture-$($Architecture.ToLowerInvariant())"
New-Item -ItemType Directory -Force -Path $build | Out-Null
$cmake = @(
    'D:/Tools/VS2022BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe',
    'C:/Program Files/Microsoft Visual Studio/2022/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe',
    'C:/Program Files/Microsoft Visual Studio/2022/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe'
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $cmake) { throw 'cmake not found in the supported Visual Studio locations' }
$vsArchitecture = if ($Architecture -eq 'x64') { 'x64' } else { 'Win32' }
& $cmake -S $PSScriptRoot -B $build -G 'Visual Studio 17 2022' -A $vsArchitecture | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'CMake configure failed' }
& $cmake --build $build --config Release --parallel 2 | Out-Host
if ($LASTEXITCODE -ne 0) { throw 'CMake build failed' }
Copy-Item -LiteralPath (Join-Path $build 'Release/envbox-management-fixture.exe') -Destination (Join-Path $repo "target/envbox-management-fixture-$($Architecture.ToLowerInvariant()).exe") -Force
