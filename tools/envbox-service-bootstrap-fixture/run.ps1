param(
    [Parameter(Mandatory = $true)][string]$RuntimeDirectory,
    [string]$ResultDirectory,
    [switch]$FreshWorker
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not $ResultDirectory) {
    $ResultDirectory = Join-Path $repo ('target/service-bootstrap-' + [guid]::NewGuid().ToString('N'))
}
$RuntimeDirectory = [IO.Path]::GetFullPath($RuntimeDirectory)
$ResultDirectory = [IO.Path]::GetFullPath($ResultDirectory)
if (-not $FreshWorker) {
    New-Item -ItemType Directory -Path $ResultDirectory -ErrorAction Stop | Out-Null
    $quote = { param($value) "'" + $value.Replace("'", "''") + "'" }
    $command = '& ' + (& $quote $PSCommandPath) + ' -RuntimeDirectory ' +
        (& $quote $RuntimeDirectory) + ' -ResultDirectory ' +
        (& $quote $ResultDirectory) + ' -FreshWorker'
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $start = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
        CommandLine = "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand $encoded"
    }
    if ($start.ReturnValue -ne 0) { throw "Fresh WMI worker create failed: $($start.ReturnValue)" }
    $done = Join-Path $ResultDirectory 'done.txt'
    $deadline = [DateTime]::UtcNow.AddMinutes(2)
    while (-not (Test-Path -LiteralPath $done)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw "Worker $($start.ProcessId) deadline exceeded; evidence: $ResultDirectory" }
        Start-Sleep -Milliseconds 200
    }
    Get-Content -LiteralPath (Join-Path $ResultDirectory 'result.json')
    exit ([int](Get-Content -LiteralPath $done))
}
$report = [ordered]@{ controller_pid = $PID; runtime_modules = -1; cases = @(); runtime_hashes = @(); exit = 1 }
try {
    Set-Location -LiteralPath $repo
    Get-ChildItem Env:ENVBOX* -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-Item -LiteralPath ('Env:' + $_.Name)
    }
    $report.runtime_modules = @((Get-Process -Id $PID).Modules |
        Where-Object ModuleName -Like 'envbox-runtime*').Count
    if ($report.runtime_modules -ne 0) { throw 'Fixture controller has Runtime loaded' }
    $rows = [Collections.Generic.List[object]]::new()
    foreach ($arch in @('64', '32')) {
        $fixture = Join-Path $repo "target/service-bootstrap-fixture$arch/Release/service-bootstrap-fixture.exe"
        $dll = Join-Path $RuntimeDirectory "envbox-runtime$arch.dll"
        if (-not (Test-Path -LiteralPath $fixture) -or -not (Test-Path -LiteralPath $dll)) { throw "Missing fixture or Runtime for $arch" }
        $hash = Get-FileHash -LiteralPath $dll
        $report.runtime_hashes += [ordered]@{ path = [string]$hash.Path; sha256 = [string]$hash.Hash }
        foreach ($mode in @('flag-missing', 'flag-one', 'flag-zero', 'flag-empty', 'flag-long',
                             'flag-one-clear', 'flag-missing-one', 'flag-invalid-one', 'pipe',
                             'fallback-legacy', 'fallback-trusted', 'fallback-invalid')) {
            $log = Join-Path $ResultDirectory "$arch-$mode.log"
            if ($mode.StartsWith('fallback-')) { & $fixture $mode $dll *> $log }
            else { & $fixture $mode *> $log }
            $code = $LASTEXITCODE
            $rows.Add([ordered]@{ arch = $arch; mode = $mode; exit = $code; log = $log; output = [IO.File]::ReadAllText($log) })
        }
    }
    $report.cases = @($rows.ToArray())
    $report.exit = [int](@($rows | Where-Object { $_.exit -ne 0 }).Count -gt 0)
} catch { $report.error = $_.Exception.Message }
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $ResultDirectory 'result.json') -Encoding UTF8
$report.exit | Set-Content -LiteralPath (Join-Path $ResultDirectory 'done.txt')
exit $report.exit
