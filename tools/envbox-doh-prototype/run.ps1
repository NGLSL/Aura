param([switch]$HostWorker)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $root
if (-not $HostWorker) {
    $cmake = Get-Command cmake.exe -ErrorAction SilentlyContinue
    if ($cmake) { $cmakePath = $cmake.Source }
    else { $cmakePath = 'D:\Tools\VS2022BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe' }
    foreach ($arch in @(@{Name='64'; Target='x64'}, @{Name='32'; Target='Win32'})) {
        & $cmakePath -S $PSScriptRoot -B ('target/doh-prototype' + $arch.Name) -G 'Visual Studio 17 2022' -A $arch.Target
        if ($LASTEXITCODE -ne 0) { throw 'CMake configure failed' }
        & $cmakePath --build ('target/doh-prototype' + $arch.Name) --config Release
        if ($LASTEXITCODE -ne 0) { throw 'CMake build failed' }
    }
    $command = 'powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -HostWorker'
    $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=$command; CurrentDirectory=$root}
    if ($created.ReturnValue -ne 0) { throw ('WMI Create failed: ' + $created.ReturnValue) }
    Write-Output ('Uninjected host runner PID=' + $created.ProcessId + '; results: target/doh-prototype-results.json')
    return
}

Get-ChildItem Env:ENVBOX* | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
$modules = (Get-Process -Id $PID).Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' }
if ($modules) { throw 'Host runner is injected' }
$outcomes = @()
foreach ($arch in @('64','32')) {
    $exe = 'target/doh-prototype' + $arch + '/Release/envbox-doh-prototype.exe'
    foreach ($test in @(@{Name='positive'; Host='cloudflare-dns.com'; IP='1.1.1.1'}, @{Name='wrong-identity'; Host='wrong.identity.invalid'; IP='1.1.1.1'}, @{Name='expired'; Host='expired.badssl.com'; IP='104.154.89.105'})) {
        $ErrorActionPreference = 'Continue'
        & $exe $test.Host $test.IP 443 *> ('target/doh-' + $arch + '-' + $test.Name + '.log')
        $exit = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        $log = Get-Content -LiteralPath ('target/doh-' + $arch + '-' + $test.Name + '.log') -Raw
        $observed = switch ($test.Name) {
            'positive' { $exit -eq 0 -and $log.Contains('remote_ip=1.1.1.1') -and $log.Contains('http_protocol_used=1') }
            'wrong-identity' { $exit -ne 0 -and $log.Contains('certificate_failure_flags=0x00000010') }
            'expired' { $exit -ne 0 -and $log.Contains('certificate_failure_flags=0x00000020') }
        }
        $outcomes += @{bits=$arch; scenario=$test.Name; exit=$exit; expected_evidence_observed=$observed}
    }
}
$python = (Get-Command python.exe).Source
$observation = Join-Path $root 'target/doh-clienthello.json'
$arguments = '"' + (Join-Path $PSScriptRoot 'observe-clienthello.py') + '" "' + $observation + '"'
$observer = Start-Process -FilePath $python -ArgumentList $arguments -WindowStyle Hidden -PassThru
Start-Sleep -Milliseconds 400
foreach ($arch in @('64','32')) {
    $ErrorActionPreference = 'Continue'
    & ('target/doh-prototype' + $arch + '/Release/envbox-doh-prototype.exe') cloudflare-dns.com 127.0.0.1 18443 *> ('target/doh-' + $arch + '-clienthello.log')
    $ErrorActionPreference = 'Stop'
}
if (-not $observer.WaitForExit(25000)) { throw 'ClientHello observer did not finish' }
@{uninjected_host=$true; os=(Get-CimInstance Win32_OperatingSystem).Version; winhttp_version=(Get-Item "$env:WINDIR/System32/winhttp.dll").VersionInfo.FileVersion; results=$outcomes; zero_host_dns='UNPROVEN'; gate='NO_GO_INCOMPLETE_OBSERVATION_AND_TARGET_MATRIX'} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath 'target/doh-prototype-results.json' -Encoding utf8
