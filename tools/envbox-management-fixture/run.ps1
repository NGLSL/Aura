param(
    [string]$Prefix = 'management-independent-final'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
if ($Prefix -notmatch '^[a-z0-9-]+$') { throw 'Invalid bounded fixture prefix' }

$runtime = @([System.Diagnostics.Process]::GetCurrentProcess().Modules |
    Where-Object ModuleName -Like 'envbox-runtime*')
$hostLog = Join-Path $repo "target/$Prefix-host.log"
$architecture = if ([Environment]::Is64BitProcess) { 'x64' } else { 'x86' }
Set-Content -LiteralPath $hostLog -Value @(
    "PID=$PID"
    "RuntimeModules=$($runtime.Count)"
    "Architecture=$architecture"
)
if ($runtime.Count -ne 0) { throw 'Host runner is injected' }

Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' |
    ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
& (Join-Path $PSScriptRoot 'build.ps1') -Architecture x64
$fixture = Join-Path $repo 'target/envbox-management-fixture-x64.exe'
$env:AURA_MANAGEMENT_FIXTURE = $fixture
$cargo = @('D:/Tools/cargo/bin/cargo.exe', 'cargo.exe') |
    Where-Object { (Test-Path -LiteralPath $_) -or (Get-Command $_ -ErrorAction SilentlyContinue) } |
    Select-Object -First 1
if (-not $cargo) { throw 'cargo not found' }
$log = Join-Path $repo "target/$Prefix.log"
$exitFile = Join-Path $repo "target/$Prefix.exit"
$stdout = "$log.stdout"
$stderr = "$log.stderr"
$arguments = @(
    '+1.99.0', 'test', '-p', 'envbox-supervisor', '--test', 'channel', '--locked', '--',
    '--ignored', '--exact', 'low_integrity_cannot_manage_medium_supervisor', '--nocapture',
    '--test-threads=1'
)
$process = Start-Process -FilePath $cargo -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$exitCode = $process.ExitCode
Get-Content -LiteralPath $stdout, $stderr | Set-Content -LiteralPath $log
Set-Content -LiteralPath $exitFile -Value $exitCode
if ($exitCode -ne 0) { throw "low-integrity management fixture failed with exit $exitCode; see $log" }

$missingEndpoint = '\\.\pipe\aura-management-fixture-missing-' + [guid]::NewGuid().ToString('N')
$missingOutput = Join-Path $repo "target/$Prefix-missing.log"
$missingProcess = Start-Process -FilePath $fixture -ArgumentList @(
    '--launch-low', $missingEndpoint, $missingOutput
) -WindowStyle Hidden -PassThru -Wait
$missingDetails = Get-Content -LiteralPath $missingOutput -Raw
if ($missingProcess.ExitCode -eq 0 -or
    $missingDetails -notmatch '(?m)^pipe_open=error$' -or
    $missingDetails -notmatch '(?m)^pipe_open_error_code=2$') {
    throw "fixture accepted a nonexistent endpoint; exit=$($missingProcess.ExitCode) details=$missingDetails"
}
Add-Content -LiteralPath $log -Value @(
    "control_missing_exit=$($missingProcess.ExitCode)"
    'control_missing_pipe_open=error'
    'control_missing_pipe_open_error_code=2'
)
exit 0
