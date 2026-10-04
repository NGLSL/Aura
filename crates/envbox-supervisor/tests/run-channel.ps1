param([int]$Repeats = 1, [string]$Prefix = 'channel-diagnose')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
Set-Location -LiteralPath $repo
$runtime = @([System.Diagnostics.Process]::GetCurrentProcess().Modules | Where-Object ModuleName -Like 'envbox-runtime*')
Set-Content -LiteralPath (Join-Path $repo "target/$Prefix-host.log") -Value "PID=$PID RuntimeModules=$($runtime.Count)"
if ($runtime.Count -ne 0) { throw 'Host fixture is injected' }
if ($Repeats -lt 1 -or $Repeats -gt 10 -or $Prefix -notmatch '^[a-z0-9-]+$') { throw 'Invalid bounded fixture arguments' }
Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
for ($iteration = 1; $iteration -le $Repeats; $iteration++) {
    $process = Start-Process -FilePath 'D:\Tools\cargo\bin\cargo.exe' -ArgumentList @('test','-p','envbox-supervisor','--test','channel','--','--nocapture','--test-threads=2') -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $repo "target/$Prefix-$iteration.log") -RedirectStandardError (Join-Path $repo "target/$Prefix-$iteration.stderr.log")
    Set-Content -LiteralPath (Join-Path $repo "target/$Prefix-$iteration.exit") -Value $process.ExitCode
    if ($process.ExitCode -ne 0) { exit $process.ExitCode }
}
exit 0
