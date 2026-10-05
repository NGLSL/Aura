param([string]$Prefix = 'mixed-recovery', [string]$Filter = 'mixed_recovery')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
Set-Location -LiteralPath $repo
if ($Prefix -notmatch '^[a-z0-9-]+$' -or $Filter -notmatch '^[a-z0-9_]+$') { throw 'Invalid fixture args' }
$runtime = @([System.Diagnostics.Process]::GetCurrentProcess().Modules | Where-Object ModuleName -Like 'envbox-runtime*')
Set-Content -LiteralPath (Join-Path $repo "target/$Prefix-host.log") -Value "PID=$PID RuntimeModules=$($runtime.Count)"
if ($runtime.Count -ne 0) { throw 'Host fixture is injected' }
Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
$env:AURA_SUPERVISOR_RUN_DLL = Join-Path $repo 'target/container-independent-final-runtime-v3/envbox-runtime64.dll'
$env:AURA_SUPERVISOR_RUN_TARGET = Join-Path $repo 'target/gate-fixture64/Release/envbox-startup-gate-entry.exe'
$cmake = 'D:\Tools\VS2022BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
foreach ($item in @(@{Name='64';Arch='x64'}, @{Name='32';Arch='Win32'})) {
    $build = Join-Path $repo "target/mixed-recovery-fixture$($item.Name)"
    $configured = Start-Process -FilePath $cmake -ArgumentList @('-S',(Join-Path $PSScriptRoot 'native-parent'),'-B',$build,'-A',$item.Arch) -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo "target/$Prefix-configure$($item.Name).log") -RedirectStandardError (Join-Path $repo "target/$Prefix-configure$($item.Name).stderr.log")
    if ($configured.ExitCode -ne 0) { exit $configured.ExitCode }
    $built = Start-Process -FilePath $cmake -ArgumentList @('--build',$build,'--config','Release') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo "target/$Prefix-build$($item.Name).log") -RedirectStandardError (Join-Path $repo "target/$Prefix-build$($item.Name).stderr.log")
    if ($built.ExitCode -ne 0) { exit $built.ExitCode }
}
$process = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--test','recovery',$Filter,'--','--ignored','--nocapture','--test-threads=1') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo "target/$Prefix.log") -RedirectStandardError (Join-Path $repo "target/$Prefix.stderr.log")
Set-Content -LiteralPath (Join-Path $repo "target/$Prefix.exit") -Value $process.ExitCode
exit $process.ExitCode
