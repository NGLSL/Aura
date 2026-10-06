param([string]$CMakePath = 'D:/Tools/VS2022BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe')
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$runtimeModules = @((Get-Process -Id $PID).Modules | Where-Object ModuleName -Match '^envbox-runtime')
if ($runtimeModules.Count -ne 0) { throw 'Run fixture validation in a host process without an injected Runtime' }
if (-not (Test-Path -LiteralPath $CMakePath)) { throw "CMake not found: $CMakePath" }
$records = @()
foreach ($arch in @('x64', 'Win32')) {
    $build = Join-Path $repo "target/envbox-policy-$arch"
    New-Item -ItemType Directory -Path $build -Force | Out-Null
    & $CMakePath -S $PSScriptRoot -B $build -A $arch *> (Join-Path $build 'configure.log')
    if ($LASTEXITCODE -ne 0) { throw "Configure failed: $arch" }
    & $CMakePath --build $build --config Release *> (Join-Path $build 'build.log')
    if ($LASTEXITCODE -ne 0) { throw "Build failed: $arch" }
    $exe = Join-Path $build 'Release/envbox-policy-fixture.exe'
    $run = Start-Process -FilePath $exe -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $build 'fixture.log') -RedirectStandardError (Join-Path $build 'fixture-error.log')
    if ($run.ExitCode -ne 0) { throw "Fixture failed: $arch, exit $($run.ExitCode)" }
    $output = (Get-Content -LiteralPath (Join-Path $build 'fixture.log') -Raw).Trim()
    if ($output -notmatch '^POLICY_FIXTURE_PASS checks=\d+ capacity=64 pointer_bits=(32|64)$') { throw "Missing fixture result: $arch" }
    $linkageExe = Join-Path $build 'Release/envbox-policy-cxx-linkage.exe'
    $linkageRun = Start-Process -FilePath $linkageExe -WindowStyle Hidden -Wait -PassThru -RedirectStandardOutput (Join-Path $build 'linkage.log') -RedirectStandardError (Join-Path $build 'linkage-error.log')
    if ($linkageRun.ExitCode -ne 0 -or (Get-Content -LiteralPath (Join-Path $build 'linkage.log') -Raw).Trim() -ne 'POLICY_CXX_LINKAGE_PASS') { throw "C++ linkage verification failed: $arch" }
    $records += [ordered]@{ architecture = $arch; exit_code = $run.ExitCode; output = $output; path = $exe; sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash; cxx_linkage_exit_code = $linkageRun.ExitCode }
}
$result = [ordered]@{ completed = (Get-Date).ToUniversalTime().ToString('o'); controller_pid = $PID; runtime_module_count = $runtimeModules.Count; scope = 'shared pure-C state machine; no driver loaded and no network filtering'; results = $records }
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $repo 'target/envbox-policy-fixture-results.json') -Encoding utf8
$result | ConvertTo-Json -Depth 6
