param(
    [switch]$HostWorker,
    [string]$RunId
)

$ErrorActionPreference = 'Stop'
# Cargo/CMake write normal progress to stderr. The build script checks native
# exit codes explicitly, and this prevents successful progress from aborting
# the WMI orchestration under PowerShell 7.
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
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

$target = Join-Path $repo 'target'
$resultPath = Join-Path $target ("doh-native-acceptance-results-{0}.json" -f $RunId)
$stableResultPath = Join-Path $target 'doh-native-acceptance-results.json'
if (Test-Path -LiteralPath $resultPath) {
    throw ('Refusing to reuse an existing run result: ' + $resultPath)
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
    & (Join-Path $PSScriptRoot 'build.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'Native probe build failed' }
    $command = 'powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -HostWorker -RunId ' + $RunId
    $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=$command; CurrentDirectory=$repo}
    if ($created.ReturnValue -ne 0) { throw ('WMI Create failed: ' + $created.ReturnValue) }
    $result = Wait-RunResult
    if ($null -eq $result) {
        $timeout = [ordered]@{
            run_id=$RunId
            completed=$false
            harness_completed=$false
            worker_pid=$created.ProcessId
            worker_exit=1
            gate=$false
            error='worker_timeout_waiting_for_unique_result'
        }
        Write-RunResult $timeout
        Copy-Item -LiteralPath $resultPath -Destination $stableResultPath -Force
        throw ('WMI native probe timed out; expected result was ' + $resultPath)
    }
    Copy-Item -LiteralPath $resultPath -Destination $stableResultPath -Force
    foreach ($bits in @('64', '32')) {
        Copy-Item -LiteralPath (Join-Path $target ("doh-native-acceptance-{0}-{1}.log" -f $RunId, $bits)) -Destination (Join-Path $target ("doh-native-acceptance-{0}.log" -f $bits)) -Force
    }
    Write-Output ('Uninjected fresh WMI probe PID=' + $created.ProcessId + '; run_id=' + $RunId + '; result=' + $resultPath)
    if ($result.completed -ne $true -or $result.worker_exit -ne 0) {
        throw ('Native probe worker failed; run_id=' + $RunId)
    }
    if ($result.gate -ne $true) {
        throw ('Native DoH positive gate=false; run_id=' + $RunId + '; inspect ' + $resultPath)
    }
    exit 0
}

try {
    Get-ChildItem Env:ENVBOX* -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-Item -LiteralPath ('Env:' + $_.Name)
    }
    $modules = (Get-Process -Id $PID).Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' }
    if ($modules) { throw 'Fresh native probe host is injected' }

    $records = @()
    $typedErrors = @()
    $wirePositive = $true
    $workerExit = 0
    foreach ($bits in @('64', '32')) {
        $exe = Join-Path $repo "target/doh-native-acceptance$bits/Release/doh-native-probe-host.exe"
        $dll = Join-Path $repo "target/doh-native-acceptance$bits/Release/doh-native-probe.dll"
        $log = Join-Path $repo ("target/doh-native-acceptance-{0}-{1}.log" -f $RunId, $bits)
        & $exe $dll *> $log
        $code = $LASTEXITCODE
        $records += [ordered]@{bits=$bits; exit=$code; log=$log}
        if ($code -ne 0) {
            $wirePositive = $false
            $workerExit = 1
        }
        if (Test-Path -LiteralPath $log) {
            $caseLines = @(Get-Content -LiteralPath $log | Where-Object { $_ -match '^case=' } | ForEach-Object { [string]$_ })
            if ($caseLines.Count -ne 4) { $wirePositive = $false }
            foreach ($line in $caseLines) {
                if ($line -notmatch 'error=0(?:\s|$)' -or $line -notmatch 'response_shape=1(?:\s|$)') {
                    $wirePositive = $false
                    $typedErrors += [string]$line
                }
            }
        } else {
            $wirePositive = $false
            $typedErrors += "missing_log=$log"
            $workerExit = 1
        }
    }

    # A direct endpoint result is useful evidence, but it is not a whole-host
    # acceptance proof. Keep the positive gate closed until global observation
    # and the remaining transport paths have their own evidence.
    $result = [ordered]@{
        run_id=$RunId
        completed=$true
        harness_completed=$true
        fresh_wmi=$true
        host_pid=$PID
        runtime_modules=0
        cases=$records
        wire_positive=$wirePositive
        typed_errors=$typedErrors
        acceptance_pass=$false
        gate=$false
        gate_reason='NATIVE_DEFAULT_TRUST_RESULT_RECORDED_NO_GLOBAL_OBSERVATION'
        worker_exit=$workerExit
    }
    Write-RunResult $result
    if ($workerExit -ne 0) { exit 1 }
    exit 0
} catch {
    $failure = [ordered]@{
        run_id=$RunId
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
