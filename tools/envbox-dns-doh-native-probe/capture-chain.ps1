param(
    [switch]$FreshWmiWorker,
    [string]$RunId
)

$ErrorActionPreference = 'Stop'
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

if ($FreshWmiWorker) {
    $RunId = Assert-WorkerRunId $RunId
} else {
    if (-not [string]::IsNullOrWhiteSpace($RunId)) {
        throw '-RunId is an internal WMI worker handoff parameter and cannot be supplied to the top-level script'
    }
    $RunId = [Guid]::NewGuid().ToString('N')
}

$target = Join-Path $repo 'target'
$output = Join-Path $target ("doh-native-chain-capture-{0}.log" -f $RunId)
$stableOutput = Join-Path $target 'doh-native-chain-capture.log'
$resultPath = Join-Path $target ("doh-native-chain-capture-results-{0}.json" -f $RunId)
$stableResultPath = Join-Path $target 'doh-native-chain-capture-results.json'
if (Test-Path -LiteralPath $resultPath) {
    throw ('Refusing to reuse an existing run result: ' + $resultPath)
}
$endpointTimeoutMs = 15000
$workerTimeoutSeconds = 60

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Cryptography.X509Certificates;

public sealed class EnvBoxNativeChainCaptureResult
{
    public string[] Lines { get; set; }
    public bool InvalidHandshake { get; set; }
    public bool TimedOut { get; set; }
}

public static class EnvBoxNativeChainCapture
{
    public static EnvBoxNativeChainCaptureResult Run(string host, string ip, int endpointTimeoutMs, int workerRemainingMs)
    {
        var lines = new List<string> { "endpoint=" + host + " literal_ip=" + ip };
        var invalid = false;
        var stopwatch = Stopwatch.StartNew();
        Func<int> budget = () => Math.Min(endpointTimeoutMs, workerRemainingMs - (int)stopwatch.ElapsedMilliseconds);
        using (var tcp = new TcpClient())
        {
            var connectBudget = budget();
            if (connectBudget <= 0)
            {
                lines.Add("worker_timeout=1 phase=tcp");
                return Result(lines, invalid, true);
            }
            var connectTask = tcp.ConnectAsync(IPAddress.Parse(ip), 443);
            bool connected;
            string connectWaitError = null;
            try
            {
                connected = connectTask.Wait(connectBudget);
            }
            catch (Exception error)
            {
                connected = true;
                connectWaitError = error.GetBaseException().Message;
            }
            if (!connected)
            {
                lines.Add("tcp_timeout=1 budget_ms=" + connectBudget);
                return Result(lines, invalid, true);
            }
            if (connectTask.IsFaulted || connectWaitError != null)
            {
                lines.Add("tcp_error=" + (connectWaitError ?? connectTask.Exception.GetBaseException().Message));
                return Result(lines, invalid, false);
            }

            var callback = new RemoteCertificateValidationCallback((sender, certificate, chain, errors) =>
            {
                using (var leaf = new X509Certificate2(certificate))
                {
                    lines.Add("validation_errors=" + errors + " leaf_subject=" + leaf.Subject + " leaf_issuer=" + leaf.Issuer + " leaf_thumbprint_sha1=" + leaf.Thumbprint);
                }
                if (chain != null)
                {
                    var index = 0;
                    foreach (var element in chain.ChainElements)
                    {
                        var cert = element.Certificate;
                        lines.Add("chain[" + index + "] subject=" + cert.Subject + " issuer=" + cert.Issuer + " thumbprint_sha1=" + cert.Thumbprint);
                        index++;
                    }
                    foreach (var status in chain.ChainStatus)
                    {
                        lines.Add("chain_status=" + status.Status + " " + status.StatusInformation.Trim());
                    }
                }
                return false;
            });
            using (var ssl = new SslStream(tcp.GetStream(), false, callback))
            {
                var tlsBudget = budget();
                if (tlsBudget <= 0)
                {
                    lines.Add("worker_timeout=1 phase=tls");
                    return Result(lines, invalid, true);
                }
                var tlsTask = ssl.AuthenticateAsClientAsync(host);
                bool authenticated;
                string tlsWaitError = null;
                try
                {
                    authenticated = tlsTask.Wait(tlsBudget);
                }
                catch (Exception error)
                {
                    authenticated = true;
                    tlsWaitError = error.GetBaseException().Message;
                }
                if (!authenticated)
                {
                    lines.Add("tls_timeout=1 budget_ms=" + tlsBudget);
                    return Result(lines, invalid, true);
                }
                if (tlsTask.IsFaulted || tlsWaitError != null)
                {
                    lines.Add("handshake_error=" + (tlsWaitError ?? tlsTask.Exception.GetBaseException().Message));
                    return Result(lines, invalid, false);
                }
                lines.Add("handshake=accepted_unexpectedly");
                invalid = true;
                return Result(lines, invalid, false);
            }
        }
    }

    private static EnvBoxNativeChainCaptureResult Result(List<string> lines, bool invalid, bool timedOut)
    {
        return new EnvBoxNativeChainCaptureResult { Lines = lines.ToArray(), InvalidHandshake = invalid, TimedOut = timedOut };
    }
}
'@

function Write-RunResult {
    param([object]$Value)
    $temporary = "$resultPath.tmp.$PID"
    $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $temporary -Encoding utf8
    Move-Item -LiteralPath $temporary -Destination $resultPath -Force
}

function Wait-RunResult {
    param([int]$TimeoutSeconds = 90)
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

function Capture-Endpoint {
    param(
        [string]$HostName,
        [string]$LiteralIp,
        [datetime]$WorkerDeadline
    )

    $remaining = [int][Math]::Floor(($WorkerDeadline - [DateTime]::UtcNow).TotalMilliseconds)
    $native = [EnvBoxNativeChainCapture]::Run($HostName, $LiteralIp, $endpointTimeoutMs, $remaining)
    return [pscustomobject]@{
        lines=@($native.Lines | ForEach-Object { [string]$_ })
        invalid=[bool]$native.InvalidHandshake
        timed_out=[bool]$native.TimedOut
    }
}

if (-not $FreshWmiWorker) {
    $command = 'powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -FreshWmiWorker -RunId ' + $RunId
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
            invalid_handshake=$false
            error='worker_timeout_waiting_for_unique_result'
        }
        Write-RunResult $timeout
        Copy-Item -LiteralPath $resultPath -Destination $stableResultPath -Force
        throw ('WMI chain capture timed out; expected result was ' + $resultPath)
    }
    Copy-Item -LiteralPath $output -Destination $stableOutput -Force -ErrorAction SilentlyContinue
    Copy-Item -LiteralPath $resultPath -Destination $stableResultPath -Force
    Write-Output ('Uninjected fresh WMI chain capture PID=' + $created.ProcessId + '; run_id=' + $RunId + '; result=' + $resultPath)
    if ($result.completed -ne $true -or $result.worker_exit -ne 0) {
        throw ('Native chain capture worker failed; run_id=' + $RunId)
    }
    exit 0
}

try {
    Get-ChildItem Env:ENVBOX* -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-Item -LiteralPath ('Env:' + $_.Name)
    }
    $modules = (Get-Process -Id $PID).Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' }
    if ($modules) { throw 'Fresh chain capture worker is injected' }
    $workerDeadline = [DateTime]::UtcNow.AddSeconds($workerTimeoutSeconds)
    $resultLines = [System.Collections.Generic.List[string]]::new()
    $records = @()
    $invalidHandshake = $false
    $timedOut = $false
    foreach ($endpoint in @(
        @{HostName='cloudflare-dns.com'; LiteralIp='1.1.1.1'},
        @{HostName='dns.google'; LiteralIp='8.8.8.8'}
    )) {
        $capture = Capture-Endpoint -HostName $endpoint.HostName -LiteralIp $endpoint.LiteralIp -WorkerDeadline $workerDeadline
        foreach ($line in $capture.lines) { [void]$resultLines.Add($line) }
        $invalidHandshake = $invalidHandshake -or $capture.invalid
        $timedOut = $timedOut -or $capture.timed_out
        $records += [ordered]@{
            host=$endpoint.HostName
            literal_ip=$endpoint.LiteralIp
            invalid_handshake=$capture.invalid
            timed_out=$capture.timed_out
        }
    }
    $resultLines | Set-Content -LiteralPath $output -Encoding utf8
    $workerExit = if ($invalidHandshake -or $timedOut) { 1 } else { 0 }
    $result = [ordered]@{
        run_id=$RunId
        completed=$true
        harness_completed=$true
        fresh_wmi=$true
        host_pid=$PID
        runtime_modules=0
        cases=$records
        invalid_handshake=$invalidHandshake
        timed_out=$timedOut
        acceptance_pass=$false
        gate=$false
        gate_reason='SCHANNEL_CHAIN_CAPTURE_ONLY'
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
