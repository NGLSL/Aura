param(
    [switch]$HostWorker,
    [switch]$AuthRootWorker,
    [string]$RunId
)

$ErrorActionPreference = 'Stop'
# Cargo writes progress to stderr even on success. Keep PowerShell from
# promoting that normal native output to a terminating exception; every native
# command below still checks `$LASTEXITCODE` explicitly.
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
Set-Location -LiteralPath $repo

function Assert-WorkerRunId {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value) -or $Value -notmatch '^[0-9a-fA-F]{32}$') {
        throw 'Worker RunId must be a 32-hex-digit GUID in N format'
    }
    $parsed = [Guid]::Empty
    if (-not [Guid]::TryParseExact($Value, 'N', [ref]$parsed)) {
        throw 'Worker RunId is not a valid GUID in N format'
    }
    return $Value.ToLowerInvariant()
}

if ($HostWorker) {
    $RunId = Assert-WorkerRunId $RunId
} else {
    if (-not [string]::IsNullOrWhiteSpace($RunId)) {
        throw '-RunId is an internal WMI worker handoff parameter and cannot be supplied to the top-level script'
    }
    $RunId = [Guid]::NewGuid().ToString('N')
}
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
$includeAuthRoot = $AuthRootWorker -or $env:DOH_DIAGNOSTIC_INCLUDE_AUTHROOT -eq '1'
if ($includeAuthRoot) {
    # WMI creates the worker with the host environment, so carry this
    # research-only selector explicitly instead of relying on inheritance.
    $env:DOH_DIAGNOSTIC_INCLUDE_AUTHROOT = '1'
}
$variantSuffix = if ($includeAuthRoot) { '-authroot' } else { '' }
$targetDir = Join-Path $repo "target/doh-native-diagnostic$variantSuffix"
$resultPrefix = "doh-native-diagnostic$variantSuffix"
$resultPath = Join-Path $repo ("target/{0}-results-{1}.json" -f $resultPrefix, $RunId)
$stableResultPath = Join-Path $repo ("target/{0}-results.json" -f $resultPrefix)
if (Test-Path -LiteralPath $resultPath) {
    throw ('Refusing to reuse an existing run result: ' + $resultPath)
}

function Invoke-NativeExitCode {
    param([scriptblock]$Command)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    return $code
}

function Write-RunResult {
    param([object]$Value)
    $temporary = "$resultPath.tmp.$PID"
    $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $temporary -Encoding utf8
    Move-Item -LiteralPath $temporary -Destination $resultPath -Force
}

function Wait-RunResult {
    param([int]$TimeoutSeconds = 180)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $resultPath) {
            try {
                $candidate = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
                if ($candidate.run_id -eq $RunId -and $null -ne $candidate.completed) {
                    return $candidate
                }
            } catch {
                # The worker may still be replacing the temporary result.
            }
        }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

if (-not $HostWorker) {
    foreach ($item in @(
        @{Bits='64'; Target='x86_64-pc-windows-msvc'},
        @{Bits='32'; Target='i686-pc-windows-msvc'}
    )) {
        $log = Join-Path $repo ("target/{0}-build-{1}-{2}.log" -f $resultPrefix, $RunId, $item.Bits)
        $code = Invoke-NativeExitCode {
            & cargo +1.99.0 build --manifest-path $manifest --target $item.Target --target-dir $targetDir --offline --locked *> $log
        }
        if ($code -ne 0) { throw "diagnostic build failed: $log (exit=$code)" }
    }
    $workerVariant = if ($includeAuthRoot) { ' -AuthRootWorker' } else { '' }
    $command = 'powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -HostWorker -RunId ' + $RunId + $workerVariant
    $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=$command; CurrentDirectory=$repo}
    if ($created.ReturnValue -ne 0) { throw ('WMI Create failed: ' + $created.ReturnValue) }
    $result = Wait-RunResult
    if ($null -eq $result) {
        $timeout = [ordered]@{
            run_id=$RunId
            variant=if ($includeAuthRoot) { 'authroot' } else { 'native' }
            completed=$false
            harness_completed=$false
            worker_pid=$created.ProcessId
            worker_exit=1
            gate=$false
            error='worker_timeout_waiting_for_unique_result'
        }
        Write-RunResult $timeout
        Copy-Item -LiteralPath $resultPath -Destination $stableResultPath -Force
        throw ('WMI diagnostic timed out; expected result was ' + $resultPath)
    }
    Copy-Item -LiteralPath $resultPath -Destination $stableResultPath -Force
    foreach ($bits in @('64', '32')) {
        Copy-Item -LiteralPath (Join-Path $repo ("target/{0}-{1}-{2}.log" -f $resultPrefix, $RunId, $bits)) -Destination (Join-Path $repo ("target/{0}-{1}.log" -f $resultPrefix, $bits)) -Force
    }
    Write-Output ('Uninjected fresh WMI diagnostic PID=' + $created.ProcessId + '; run_id=' + $RunId + '; result=' + $resultPath)
    if ($result.completed -ne $true -or $result.worker_exit -ne 0) {
        throw ('Native diagnostic worker failed; run_id=' + $RunId)
    }
    # `gate=false` is intentional for this diagnostic: typed certificate and
    # network errors are valid observations, not a positive acceptance claim.
    exit 0
}

try {
    Get-ChildItem Env:ENVBOX* -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-Item -LiteralPath ('Env:' + $_.Name)
    }
    $modules = (Get-Process -Id $PID).Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' }
    if ($modules) { throw 'Fresh diagnostic host is injected' }
    $records = @()
    $typedResults = @()
    $workerExit = 0
    foreach ($item in @(
        @{Bits='64'; Target='x86_64-pc-windows-msvc'},
        @{Bits='32'; Target='i686-pc-windows-msvc'}
    )) {
        $exe = Join-Path $targetDir "$($item.Target)/debug/envbox-dns-doh-native-diagnostic.exe"
        $log = Join-Path $repo ("target/{0}-{1}-{2}.log" -f $resultPrefix, $RunId, $item.Bits)
        & $exe *> $log
        $code = $LASTEXITCODE
        $records += [ordered]@{bits=$item.Bits; exit=$code; log=$log}
        if ($code -ne 0) { $workerExit = 1 }
        if (Test-Path -LiteralPath $log) {
            $typedResults += @(Get-Content -LiteralPath $log | Where-Object {
                $_ -match '^case=' -or $_ -match '^(tls_error|tcp_error|.*timeout|snapshot_or_verifier_error|peer_chain=|ssl_root_policy=)'
            } | ForEach-Object { [string]$_ })
        } else {
            $typedResults += "missing_log=$log"
            $workerExit = 1
        }
    }
    $result = [ordered]@{
        run_id=$RunId
        variant=if ($includeAuthRoot) { 'authroot' } else { 'native' }
        completed=$true
        harness_completed=$true
        fresh_wmi=$true
        host_pid=$PID
        runtime_modules=0
        cases=$records
        typed_results=$typedResults
        acceptance_pass=$false
        gate=$false
        gate_reason='DIAGNOSTIC_TYPED_RESULT_ONLY_NO_POSITIVE_ACCEPTANCE'
        worker_exit=$workerExit
    }
    Write-RunResult $result
    if ($workerExit -ne 0) { exit 1 }
    exit 0
} catch {
    $failure = [ordered]@{
        run_id=$RunId
        variant=if ($includeAuthRoot) { 'authroot' } else { 'native' }
        completed=$false
        harness_completed=$false
        fresh_wmi=$true
        host_pid=$PID
        runtime_modules=0
        acceptance_pass=$false
        gate=$false
        worker_exit=1
        error=$_.Exception.Message
    }
    try { Write-RunResult $failure } catch { }
    exit 1
}
