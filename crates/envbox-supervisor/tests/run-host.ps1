# Run the management fixture from a host process, outside Aura injection.
# The caller may use Win32_Process.Create in this development environment.
param([switch]$RunTransaction, [switch]$StopScope, [switch]$JobLifetime, [switch]$Recovery)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
Set-Location -LiteralPath $repo
$runtime = @([System.Diagnostics.Process]::GetCurrentProcess().Modules | Where-Object ModuleName -Like 'envbox-runtime*')
Set-Content -LiteralPath (Join-Path $repo 'target/supervisor-host-facts.log') -Value "PID=$PID RuntimeModules=$($runtime.Count)"
if ($runtime.Count -ne 0) { throw 'Host fixture is injected; refusing management positive tests' }
Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
$cargoProcess = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--tests','--','--test-threads=1') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo 'target/supervisor-host-test.log') -RedirectStandardError (Join-Path $repo 'target/supervisor-host-test.stderr.log')
$testExit = $cargoProcess.ExitCode
if ($testExit -eq 0 -and $RunTransaction) {
    $env:AURA_SUPERVISOR_RUN_DLL = Join-Path $repo 'target/container-independent-final-runtime-v2/envbox-runtime64.dll'
    $env:AURA_SUPERVISOR_RUN_TARGET = Join-Path $repo 'target/gate-fixture64/Release/envbox-startup-gate-entry.exe'
    $transaction = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--test','run_transaction','--','--ignored','--nocapture','--test-threads=1') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo 'target/supervisor-run-test.log') -RedirectStandardError (Join-Path $repo 'target/supervisor-run-test.stderr.log')
    $testExit = $transaction.ExitCode
    if ($testExit -eq 0 -and $StopScope) {
        $scope = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--test','stop_scope','--','--ignored','--nocapture','--test-threads=1') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo 'target/supervisor-stop-test.log') -RedirectStandardError (Join-Path $repo 'target/supervisor-stop-test.stderr.log')
        $testExit = $scope.ExitCode
    }
}
if ($testExit -eq 0 -and $JobLifetime) {
    $env:AURA_SUPERVISOR_RUN_TARGET = Join-Path $repo 'target/gate-fixture64/Release/envbox-startup-gate-entry.exe'
    $lifetime = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--test','job_lifetime','--','--ignored','--nocapture','--test-threads=1') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo 'target/supervisor-job-lifetime-test.log') -RedirectStandardError (Join-Path $repo 'target/supervisor-job-lifetime-test.stderr.log')
    $testExit = $lifetime.ExitCode
}
Set-Content -LiteralPath (Join-Path $repo 'target/supervisor-host-test.exit') -Value $testExit
if ($testExit -eq 0 -and $Recovery) {
    $env:AURA_SUPERVISOR_RUN_DLL = Join-Path $repo 'target/container-independent-final-runtime-v2/envbox-runtime64.dll'
    $env:AURA_SUPERVISOR_RUN_TARGET = Join-Path $repo 'target/gate-fixture64/Release/envbox-startup-gate-entry.exe'
    $recovery = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--test','recovery','supervisor_crash','--','--ignored','--nocapture','--test-threads=1') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo 'target/supervisor-recovery-test.log') -RedirectStandardError (Join-Path $repo 'target/supervisor-recovery-test.stderr.log')
    $testExit = $recovery.ExitCode
    Set-Content -LiteralPath (Join-Path $repo 'target/supervisor-recovery-test.exit') -Value $testExit
}
exit $testExit
