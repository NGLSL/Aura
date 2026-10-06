param(
    [switch]$HostWorker,
    [string]$RunId,
    [switch]$IncludeIPv6
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
    param([int]$TimeoutSeconds = 240)
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
    if ($IncludeIPv6) { $command += ' -IncludeIPv6' }
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
    if ($result.native_ipv4_pass -ne $true -or $result.policy_negative_pass -ne $true -or $result.process_api_pass -ne $true -or $result.config_policy_pass -ne $true) {
        throw ('Native DoH IPv4/policy/process API validation failed; run_id=' + $RunId + '; inspect ' + $resultPath)
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
    $nativeIpv4Pass = $true
    $policyNegativePass = $true
    $processApiPass = $true
    $configPolicyPass = $true
    $ipv6Unexecuted = @()
    $workerExit = 0
    foreach ($bits in @('64', '32')) {
        $configProbe = Join-Path $repo "target/doh-native-acceptance$bits/Release/config-policy-probe.exe"
        $configLog = Join-Path $target ("doh-native-config-policy-{0}-{1}.log" -f $RunId, $bits)
        & $configProbe *> $configLog
        $configExit = $LASTEXITCODE
        $configLines = @(Get-Content -LiteralPath $configLog)
        $configPassed = $configExit -eq 0 -and
            @($configLines | Where-Object { $_ -match '^case=\S+ result=PASS$' }).Count -eq 7 -and
            @($configLines | Where-Object { $_ -match '^failures=0$' }).Count -eq 1 -and
            @($configLines | Where-Object { $_ -match '^probe_pid=\d+ runtime_modules=0$' }).Count -eq 1
        if (-not $configPassed) { $configPolicyPass = $false; $workerExit = 1 }
        $exe = Join-Path $repo "target/doh-native-acceptance$bits/Release/doh-native-probe-host.exe"
        $dll = Join-Path $repo "target/doh-native-acceptance$bits/Release/doh-native-probe.dll"
        $log = Join-Path $repo ("target/doh-native-acceptance-{0}-{1}.log" -f $RunId, $bits)
        $trap = Join-Path $repo "target/doh-api-trap$bits/Release/envbox-doh-api-trap.dll"
        if (-not (Test-Path -LiteralPath $trap)) { throw ('Build the process-local fixture API trap first: ' + $trap) }
        # The existing trap has an immutable single endpoint allowance. Each
        # case therefore gets a separate host process and a fresh snapshot.
        $architectureCases = @()
        for ($index = 0; $index -lt 7; $index++) {
            if (-not $IncludeIPv6 -and $index -in @(1, 3)) {
                $ipv6Unexecuted += "bits=$bits case=$index executed=0 reason=deferred_by_user"
                continue
            }
            $caseLog = Join-Path $repo ("target/doh-native-acceptance-{0}-{1}-case{2}.log" -f $RunId, $bits, $index)
            & $exe $dll $index $trap *> $caseLog
            $code = $LASTEXITCODE
            $architectureCases += [ordered]@{index=$index; exit=$code; log=$caseLog}
            Get-Content -LiteralPath $caseLog | Add-Content -LiteralPath $log
            if ($code -ne 0) { $workerExit = 1 }
            $lines = @(Get-Content -LiteralPath $caseLog)
            $caseLines = @($lines | Where-Object { $_ -match '^case=' })
            $trapLines = @($lines | Where-Object { $_ -match '^trap=' })
            if ($caseLines.Count -ne 1 -or $trapLines.Count -ne 1 -or $code -ne 0) {
                $nativeIpv4Pass = $false; $policyNegativePass = $false; $processApiPass = $false; $wirePositive = $false
                continue
            }
            $line = [string]$caseLines[0]
            $executed = $line -match 'executed=1(?:\s|$)'
            $positive = $executed -and $line -match 'error=0(?:\s|$)' -and $line -match 'response_shape=1(?:\s|$)' -and $line -match 'rcode=0(?:\s|$)'
            if ($index -lt 4 -and -not $positive) { $wirePositive = $false }
            if ($index -in @(0, 2) -and -not $positive) { $nativeIpv4Pass = $false }
            if ($index -in @(1, 3) -and -not $executed) { $ipv6Unexecuted += "bits=$bits $line" }
            if ($index -in @(4, 5) -and (-not $executed -or $line -notmatch 'error=7(?:\s|$)' -or $line -notmatch 'length=0(?:\s|$)')) { $policyNegativePass = $false }
            if ($index -eq 6 -and (-not $executed -or $line -notmatch 'error=1(?:\s|$)' -or $line -notmatch 'length=0(?:\s|$)')) { $policyNegativePass = $false }
            if (-not $positive -and $executed) { $typedErrors += $line }
            $counters = ([string]$trapLines[0]).Substring(5) | ConvertFrom-Json
            $forbidden = @($counters.apis | Where-Object { $_.name -notin @('connect', 'WSAConnect', 'WSAIoctl', 'ConnectEx') -and $_.calls -ne 0 })
            if ($counters.installed -ne $true -or $forbidden.Count -ne 0 -or $counters.connects_denied -ne 0 -or $counters.extension_denied -ne 0) { $processApiPass = $false }
            if ($index -eq 6 -and ($counters.connects_allowed -ne 0 -or @($counters.apis | Where-Object { $_.calls -ne 0 }).Count -ne 0)) { $policyNegativePass = $false }
        }
        $records += [ordered]@{
            bits=$bits; log=$log; cases=$architectureCases
            config_policy=[ordered]@{
                path=$configProbe; sha256=(Get-FileHash -LiteralPath $configProbe).Hash
                exit=$configExit; pass=$configPassed; log=$configLog
            }
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
        native_ipv4_pass=$nativeIpv4Pass
        policy_negative_pass=$policyNegativePass
        process_api_pass=$processApiPass
        config_policy_pass=$configPolicyPass
        ipv6_deferred=(-not $IncludeIPv6)
        observation_scope='current_process_24_API_tripwire_single_literal_TCP_endpoint_no_global_packet_capture'
        ipv6_unexecuted=$ipv6Unexecuted
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
