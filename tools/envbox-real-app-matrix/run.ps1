param(
    [switch]$HostWorker,
    [int]$WaitSeconds = 180,
    [switch]$SaveJsonSelfTest
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo

$resultPath = Join-Path $repo 'target/real-app-matrix-final.json'
$logDir = Join-Path $repo 'target/real-app-matrix'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

function Save-Json([object]$value, [string]$path) {
    $fullPath = [IO.Path]::GetFullPath($path)
    $directory = [IO.Path]::GetDirectoryName($fullPath)
    $fileName = [IO.Path]::GetFileName($fullPath)
    $tempPath = Join-Path $directory ('.' + $fileName + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    $backupPath = $tempPath + '.backup'
    $json = $value | ConvertTo-Json -Depth 12
    try {
        [IO.File]::WriteAllText($tempPath, $json, (New-Object System.Text.UTF8Encoding -ArgumentList $false))
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            [IO.File]::Replace($tempPath, $path, $backupPath, $true)
        } else {
            [IO.File]::Move($tempPath, $path)
        }
    } finally {
        if (Test-Path -LiteralPath $tempPath -PathType Leaf) {
            Remove-Item -LiteralPath $tempPath -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $backupPath -PathType Leaf) {
            Remove-Item -LiteralPath $backupPath -Force -ErrorAction SilentlyContinue
        }
    }
}

if ($SaveJsonSelfTest) {
    $selfTestPath = Join-Path $logDir 'save-json-self-test.json'
    try {
        Save-Json ([ordered]@{ schema = 1; marker = 'atomic-save-json-self-test' }) $selfTestPath
        Save-Json ([ordered]@{ schema = 1; marker = 'atomic-save-json-self-test-replaced' }) $selfTestPath
        $roundTrip = Get-Content -LiteralPath $selfTestPath -Raw | ConvertFrom-Json
        if ($roundTrip.schema -ne 1 -or $roundTrip.marker -ne 'atomic-save-json-self-test-replaced') {
            throw 'Save-Json self-test round-trip mismatch'
        }
        Write-Output 'SAVE_JSON_SELF_TEST_OK'
    } finally {
        Remove-Item -LiteralPath $selfTestPath -Force -ErrorAction SilentlyContinue
    }
    exit 0
}

if (-not $HostWorker) {
    # The ordinary Codex shell may itself be inside an injected tree.  Run the
    # actual matrix from a fresh WMI-created PowerShell process and only read
    # the JSON handoff.  The worker removes inherited ENVBOX_* variables before
    # it starts any target.
    Remove-Item -LiteralPath $resultPath -Force -ErrorAction SilentlyContinue
    $command = 'powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -HostWorker'
    $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
        CommandLine = $command
        CurrentDirectory = $repo
    }
    if ($created.ReturnValue -ne 0) {
        throw "WMI worker creation failed: $($created.ReturnValue)"
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($WaitSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $resultPath) {
            $text = Get-Content -LiteralPath $resultPath -Raw
            if ($text.Trim()) {
                try {
                    $summary = $text | ConvertFrom-Json
                    Write-Output $text
                    if ($summary.exit_code -ne 0) { exit [int]$summary.exit_code }
                    return
                } catch {
                    $lastJsonError = $_.Exception.Message
                }
            }
        }
        Start-Sleep -Milliseconds 250
    }
    if ($lastJsonError) { throw "matrix result was not valid JSON before timeout: $lastJsonError" }
    throw "matrix worker did not publish $resultPath within $WaitSeconds seconds"
}

# This script is deliberately a host-side acceptance harness.  It must never
# be run from an existing Aura Runtime process and must never use the user's
# browser profile.  A WMI fresh worker is the host control for that reason.
Get-ChildItem Env:ENVBOX* -ErrorAction SilentlyContinue | ForEach-Object {
    Remove-Item -LiteralPath ('Env:' + $_.Name) -ErrorAction SilentlyContinue
}
$workerModules = @((Get-Process -Id $PID).Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' })
if ($workerModules.Count -ne 0) {
    throw "fresh worker already has Runtime modules: $($workerModules.ModuleName -join ',')"
}

$runId = [guid]::NewGuid().ToString('N')
$root = Join-Path ([System.IO.Path]::GetTempPath()) "envbox-real-app-matrix-$runId"
$configRoot = Join-Path $root 'config'
$browserProfile = Join-Path $root 'chrome-profile'
$outputRoot = Join-Path $root 'outputs'
New-Item -ItemType Directory -Force -Path $configRoot,$browserProfile,$outputRoot | Out-Null
$tracePath = Join-Path $logDir 'worker-trace.log'
Set-Content -LiteralPath $tracePath -Value "worker-start pid=$PID root=$root" -Encoding utf8

$envbox = Join-Path $repo 'target/debug/envbox.exe'
$probe = Join-Path $repo 'target/debug/envbox-probe.exe'
$browserProbe = Join-Path $repo 'target/debug/envbox-browser-probe.exe'
$runtime64 = Join-Path $repo 'target/container-independent-final-runtime-v3/envbox-runtime64.dll'
$runtime32 = Join-Path $repo 'target/container-independent-final-runtime-v3/envbox-runtime32.dll'
$missing = @($envbox,$probe,$browserProbe,$runtime64,$runtime32 | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
if ($missing.Count -ne 0) {
    throw "required artifact missing: $($missing -join ', ')"
}

if (-not ('EnvBoxRealAppMatrix.NativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace EnvBoxRealAppMatrix {
    public static class NativeMethods {
        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern IntPtr OpenProcess(uint desiredAccess, [MarshalAs(UnmanagedType.Bool)] bool inheritHandle, uint processId);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetProcessTimes(IntPtr hProcess, out long creationTime, out long exitTime, out long kernelTime, out long userTime);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool TerminateProcess(IntPtr hProcess, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool CloseHandle(IntPtr hObject);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetExitCodeProcess(IntPtr hProcess, out uint lpExitCode);
    }
}
'@
}
$script:ExitCodeApiType = 'EnvBoxRealAppMatrix.NativeMethods' -as [type]

function Preserve-Artifact([string]$source, [string]$name) {
    if (Test-Path -LiteralPath $source -PathType Leaf) {
        Copy-Item -LiteralPath $source -Destination (Join-Path $logDir $name) -Force
    }
}

function Read-OwnedExitCode([IntPtr]$handle) {
    $result = [ordered]@{
        available = $false
        code = $null
        still_active = $null
        win32_error = $null
    }
    if ($handle -eq [IntPtr]::Zero) {
        $result.win32_error = 'no-owned-process-handle'
        return $result
    }
    $code = [uint32]0
    try {
        $ok = $script:ExitCodeApiType::GetExitCodeProcess($handle, [ref]$code)
        if ($ok) {
            $result.available = $true
            $result.code = [uint64]$code
            $result.still_active = ($code -eq [uint32]259)
        } else {
            $result.win32_error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        }
    } catch {
        $result.win32_error = $_.Exception.Message
    }
    return $result
}

function Compact-RunResult([object]$result) {
    if ($null -eq $result) { return $null }
    $compact = [ordered]@{}
    foreach ($key in $result.Keys) {
        if ($key -eq 'stdout' -or $key -eq 'stderr') {
            $value = [string]$result[$key]
            $compact[$key + '_length'] = $value.Length
            $compact[$key + '_excerpt'] = if ($value.Length -gt 4096) { $value.Substring(0, 4096) + "`n[truncated; raw artifact is in target/real-app-matrix]" } else { $value }
        } else {
            $compact[$key] = $result[$key]
        }
    }
    return $compact
}

function Read-ProcessModuleFacts([int]$processId) {
    $facts = [ordered]@{
        pid = $processId
        process_name = $null
        runtime_modules = @()
        runtime_count = 0
        access = 'unknown'
    }
    try {
        $p = Get-Process -Id $processId -ErrorAction Stop
        $facts.process_name = $p.ProcessName
        $mods = @($p.Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' })
        $facts.runtime_modules = @($mods | ForEach-Object {
            $hash = $null
            if ($_.FileName -and (Test-Path -LiteralPath $_.FileName -PathType Leaf)) {
                try { $hash = (Get-FileHash -LiteralPath $_.FileName -Algorithm SHA256).Hash } catch { $hash = "HASH_ERROR:$($_.Exception.Message)" }
            }
            [ordered]@{ name = $_.ModuleName; path = $_.FileName; sha256 = $hash }
        })
        $facts.runtime_count = $facts.runtime_modules.Count
        $facts.access = 'verified'
    } catch {
        $facts.access = "unverified:$($_.Exception.Message)"
    }
    return $facts
}

function Read-ProcessCreationStamp([int]$processId) {
    $access = [uint32]0x1000 # PROCESS_QUERY_LIMITED_INFORMATION
    $handle = [IntPtr]::Zero
    try {
        $handle = $script:ExitCodeApiType::OpenProcess($access, $false, [uint32]$processId)
        if ($handle -eq [IntPtr]::Zero) { return $null }
        $creation = [int64]0
        $exit = [int64]0
        $kernel = [int64]0
        $user = [int64]0
        if (-not $script:ExitCodeApiType::GetProcessTimes($handle, [ref]$creation, [ref]$exit, [ref]$kernel, [ref]$user)) { return $null }
        return Convert-HandleFileTimeToStamp $creation
    } catch { return $null }
    finally {
        if ($handle -ne [IntPtr]::Zero) { [void]$script:ExitCodeApiType::CloseHandle($handle) }
    }
}

function Convert-HandleFileTimeToStamp([object]$fileTime) {
    return [DateTime]::FromFileTimeUtc([int64]$fileTime).Ticks.ToString()
}

function Read-ProcessTree([int]$rootPid, [switch]$IncludeModules) {
    $rows = @()
    try { $all = @(Get-CimInstance Win32_Process -ErrorAction Stop) } catch { return $rows }
    $known = [System.Collections.Generic.HashSet[int]]::new()
    [void]$known.Add($rootPid)
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($p in $all) {
            $parent = [int]$p.ParentProcessId
            if ($known.Contains($parent) -and -not $known.Contains([int]$p.ProcessId)) {
                [void]$known.Add([int]$p.ProcessId)
                $changed = $true
            }
        }
    }
    foreach ($p in $all | Where-Object { $known.Contains([int]$_.ProcessId) } | Sort-Object ProcessId) {
        $facts = if ($IncludeModules) {
            Read-ProcessModuleFacts ([int]$p.ProcessId)
        } else {
            [ordered]@{ runtime_count = $null; runtime_modules = @(); access = 'not-collected' }
        }
        $rows += [ordered]@{
            pid = [int]$p.ProcessId
            parent_pid = [int]$p.ParentProcessId
            name = $p.Name
            command_line = $p.CommandLine
            runtime_count = $facts.runtime_count
            runtime_modules = $facts.runtime_modules
            module_access = $facts.access
        }
    }
    return $rows
}

function Quote-ProcessArgument([string]$value) {
    if ($null -eq $value -or $value.Length -eq 0) { return '""' }
    if ($value -notmatch '[\s"]') { return $value }
    return '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
}

function Invoke-EnvBox([string]$name, [string]$runtime, [string[]]$arguments, [int]$timeoutMs = 30000, [switch]$CaptureTree) {
    $outPath = Join-Path $outputRoot "$name.stdout.txt"
    $errPath = Join-Path $outputRoot "$name.stderr.txt"
    Remove-Item -LiteralPath $outPath,$errPath -Force -ErrorAction SilentlyContinue

    $oldConfig = $env:ENVBOX_CONFIG_ROOT
    $oldRuntime = $env:ENVBOX_RUNTIME_DLL
    $env:ENVBOX_CONFIG_ROOT = $configRoot
    $env:ENVBOX_RUNTIME_DLL = $runtime
    try {
        $argLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' '
        $p = Start-Process -FilePath $envbox -ArgumentList $argLine -WorkingDirectory $repo -WindowStyle Hidden -RedirectStandardOutput $outPath -RedirectStandardError $errPath -PassThru
        $ownedHandle = [IntPtr]::Zero
        $handleOpened = $false
        $handleOpenError = $null
        $initialExit = $null
        try {
            # Capture the owned Process handle immediately.  Do not reopen by
            # PID later: a short-lived wrapper can otherwise race PID reuse.
            $ownedHandle = $p.Handle
            $handleOpened = ($ownedHandle -ne [IntPtr]::Zero)
            if ($handleOpened) { $initialExit = Read-OwnedExitCode $ownedHandle }
        } catch {
            $handleOpenError = $_.Exception.Message
        }
        $startedAt = [DateTime]::UtcNow
        $observedPid = $null
        $observedCreationStamp = $null
        $initialTree = @()
        while (-not $p.HasExited -and ([DateTime]::UtcNow - $startedAt).TotalMilliseconds -lt [Math]::Min($timeoutMs, 10000)) {
            if (Test-Path -LiteralPath $errPath) {
                $stderr = Get-Content -LiteralPath $errPath -Raw -ErrorAction SilentlyContinue
                if ($stderr -match 'started pid=(\d+)') {
                    $observedPid = [int]$Matches[1]
                    $observedCreationStamp = Read-ProcessCreationStamp $observedPid
                    if ($CaptureTree) { $initialTree = @(Read-ProcessTree $observedPid -IncludeModules) }
                    break
                }
            }
            Start-Sleep -Milliseconds 100
        }
        if ($CaptureTree -and $observedPid) {
            Start-Sleep -Milliseconds 1200
            $initialTree = @(Read-ProcessTree $observedPid -IncludeModules)
        }
        $waitedForExit = $p.WaitForExit($timeoutMs)
        $timedOut = -not $waitedForExit
        if ($timedOut) {
            if ($observedPid) { Stop-OwnedTree $observedPid $observedCreationStamp | Out-Null }
            try { $p.Kill() } catch { }
            [void]$p.WaitForExit(5000)
        }
        $p.Refresh()
        $stdout = if (Test-Path -LiteralPath $outPath) { Get-Content -LiteralPath $outPath -Raw } else { '' }
        $stderr = if (Test-Path -LiteralPath $errPath) { Get-Content -LiteralPath $errPath -Raw } else { '' }
        Preserve-Artifact $outPath "$name.stdout.txt"
        Preserve-Artifact $errPath "$name.stderr.txt"
        $finalExit = if ($handleOpened) { Read-OwnedExitCode $ownedHandle } else { $null }
        $wrapperExitObserved = [bool]($finalExit -and $finalExit.available -and -not $finalExit.still_active)
        $exitCode = if ($wrapperExitObserved) { [int]$finalExit.code } else { $null }
        return [ordered]@{
            name = $name
            command = ($envbox + ' ' + $argLine)
            exit_code = $exitCode
            exit_code_source = 'GetExitCodeProcess(owned_wrapper_handle)'
            wrapper_exit_observed = $wrapperExitObserved
            timed_out = $timedOut
            wrapper_handle_opened = $handleOpened
            wrapper_handle_open_error = $handleOpenError
            initial_exit = $initialExit
            final_exit = $finalExit
            wrapper_pid = $p.Id
            target_pid = $observedPid
            target_creation_stamp = $observedCreationStamp
            stdout = $stdout
            stderr = $stderr
            runtime_tree = $initialTree
            log_stdout = $outPath
            log_stderr = $errPath
        }
    } finally {
        if ($null -eq $oldConfig) { Remove-Item Env:ENVBOX_CONFIG_ROOT -ErrorAction SilentlyContinue } else { $env:ENVBOX_CONFIG_ROOT = $oldConfig }
        if ($null -eq $oldRuntime) { Remove-Item Env:ENVBOX_RUNTIME_DLL -ErrorAction SilentlyContinue } else { $env:ENVBOX_RUNTIME_DLL = $oldRuntime }
    }
}

function Stop-OwnedTree([int]$rootPid, [string]$expectedCreationStamp) {
    # OpenProcess is the only PID lookup.  Once the handle is open, the
    # creation time check and TerminateProcess use that same kernel object;
    # there is no Stop-Process/PID re-open TOCTOU window.  Descendants are
    # intentionally not walked or force-killed.
    if (-not $expectedCreationStamp) {
        return [ordered]@{ stopped = $false; reason = 'creation-stamp-missing; refused'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $null; handle_opened = $false }
    }
    $access = [uint32](0x0001 -bor 0x1000) # PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION
    $handle = [IntPtr]::Zero
    try {
        $handle = $script:ExitCodeApiType::OpenProcess($access, $false, [uint32]$rootPid)
    } catch {
        return [ordered]@{ stopped = $false; reason = 'OpenProcess exception: ' + $_.Exception.Message; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $null; handle_opened = $false }
    }
    if ($handle -eq [IntPtr]::Zero) {
        $winError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        $present = $null
        try { $present = (@(Get-CimInstance Win32_Process -Filter "ProcessId = $rootPid" -ErrorAction Stop).Count -gt 0) } catch { $present = $null }
        if ($present -eq $false) {
            return [ordered]@{ stopped = $true; reason = 'root-already-exited; no descendant walk'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $null; handle_opened = $false; open_win32_error = $winError }
        }
        return [ordered]@{ stopped = $false; reason = if ($present -eq $true) { 'OpenProcess failed; possible PID reuse or access denied' } else { 'OpenProcess failed; process presence unverified' }; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $null; handle_opened = $false; open_win32_error = $winError }
    }
    $actual = $null
    try {
        $creation = [int64]0
        $exit = [int64]0
        $kernel = [int64]0
        $user = [int64]0
        $timesOk = $script:ExitCodeApiType::GetProcessTimes($handle, [ref]$creation, [ref]$exit, [ref]$kernel, [ref]$user)
        if (-not $timesOk) {
            return [ordered]@{ stopped = $false; reason = 'GetProcessTimes failed; refused'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $null; handle_opened = $true; times_win32_error = [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
        }
        $actual = Convert-HandleFileTimeToStamp $creation
        if ($actual -ne $expectedCreationStamp) {
            return [ordered]@{ stopped = $false; reason = 'creation-stamp-mismatch; refused'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $actual; handle_opened = $true }
        }
        $terminated = $script:ExitCodeApiType::TerminateProcess($handle, [uint32]1)
        if (-not $terminated) {
            return [ordered]@{ stopped = $false; reason = 'TerminateProcess failed; refused'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $actual; handle_opened = $true; terminate_win32_error = [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
        }
        for ($round = 0; $round -lt 20; $round++) {
            $finalExit = Read-OwnedExitCode $handle
            if ($finalExit.available -and -not $finalExit.still_active) {
                return [ordered]@{ stopped = $true; reason = 'terminated; exit confirmed on owned handle; descendants left to Aura Job lifecycle'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $actual; handle_opened = $true; final_exit = $finalExit }
            }
            if (-not $finalExit.available) {
                return [ordered]@{ stopped = $false; reason = 'exit confirmation failed; refused'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $actual; handle_opened = $true; final_exit = $finalExit }
            }
            Start-Sleep -Milliseconds 250
        }
        return [ordered]@{ stopped = $false; reason = 'root-still-running-after-timeout'; root_pid = $rootPid; expected = $expectedCreationStamp; actual = $actual; handle_opened = $true; final_exit = (Read-OwnedExitCode $handle) }
    } finally {
        [void]$script:ExitCodeApiType::CloseHandle($handle)
    }
}

function Read-ProcessesUsingPath([string]$path) {
    $needle = [IO.Path]::GetFullPath($path)
    try {
        return @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
            $_.CommandLine -and $_.CommandLine.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0
        } | ForEach-Object {
            [ordered]@{ pid = [int]$_.ProcessId; parent_pid = [int]$_.ParentProcessId; name = $_.Name; command_line = $_.CommandLine }
        })
    } catch {
        return @([ordered]@{ error = $_.Exception.Message })
    }
}

function Invoke-HostBrowser([string]$browserPath, [string]$profilePath) {
    $name = 'host-browser-control'
    $outPath = Join-Path $outputRoot "$name.stdout.txt"
    $errPath = Join-Path $outputRoot "$name.stderr.txt"
    Remove-Item -LiteralPath $outPath,$errPath -Force -ErrorAction SilentlyContinue
    $arguments = @('--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--user-data-dir',$profilePath,'--remote-debugging-port=0','about:blank')
    $argLine = ($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' '
    $p = Start-Process -FilePath $browserPath -ArgumentList $argLine -WorkingDirectory $repo -WindowStyle Hidden -RedirectStandardOutput $outPath -RedirectStandardError $errPath -PassThru
    $ownedHandle = [IntPtr]::Zero
    $handleOpened = $false
    $handleOpenError = $null
    $initialExit = $null
    try {
        # Keep the direct host browser's own handle too; do not reopen by PID.
        $ownedHandle = $p.Handle
        $handleOpened = ($ownedHandle -ne [IntPtr]::Zero)
        if ($handleOpened) { $initialExit = Read-OwnedExitCode $ownedHandle }
    } catch {
        $handleOpenError = $_.Exception.Message
    }
    $targetPid = $p.Id
    $creationStamp = Read-ProcessCreationStamp $targetPid
    Start-Sleep -Milliseconds 1200
    $initialTree = @(Read-ProcessTree $targetPid -IncludeModules)
    $waitedForExit = $p.WaitForExit(15000)
    $timedOut = -not $waitedForExit
    $stop = $null
    if ($timedOut) {
        $stop = Stop-OwnedTree $targetPid $creationStamp
        try { $p.Kill() } catch { }
        [void]$p.WaitForExit(5000)
    }
    $finalExit = if ($handleOpened) { Read-OwnedExitCode $ownedHandle } else { $null }
    $wrapperExitObserved = [bool]($finalExit -and $finalExit.available -and -not $finalExit.still_active)
    $exitCode = if ($wrapperExitObserved) { [int]$finalExit.code } else { $null }
    $stdout = if (Test-Path -LiteralPath $outPath) { Get-Content -LiteralPath $outPath -Raw } else { '' }
    $stderr = if (Test-Path -LiteralPath $errPath) { Get-Content -LiteralPath $errPath -Raw } else { '' }
    Preserve-Artifact $outPath "$name.stdout.txt"
    Preserve-Artifact $errPath "$name.stderr.txt"
    $afterStamp = Read-ProcessCreationStamp $targetPid
    return [ordered]@{
        browser = $browserPath
        profile = $profilePath
        command = ($browserPath + ' ' + $argLine)
        target_pid = $targetPid
        target_creation_stamp = $creationStamp
        wrapper_handle_opened = $handleOpened
        wrapper_handle_open_error = $handleOpenError
        initial_exit = $initialExit
        final_exit = $finalExit
        wrapper_exit_observed = $wrapperExitObserved
        timed_out = $timedOut
        exit_code = $exitCode
        exit_code_source = 'GetExitCodeProcess(owned_browser_handle)'
        initial_tree = $initialTree
        remaining_tree = @(Read-ProcessTree $targetPid)
        processes_using_profile = @(Read-ProcessesUsingPath $profilePath)
        target_creation_stamp_after = $afterStamp
        target_exists_after_stop = ($null -ne $afterStamp -and $afterStamp -eq $creationStamp)
        stop = $stop
        stdout = $stdout
        stderr = $stderr
        log_stdout = $outPath
        log_stderr = $errPath
    }
}

function Invoke-StopOwnedTreeSafetyCheck {
    $outPath = Join-Path $outputRoot 'stop-safety.stdout.txt'
    $errPath = Join-Path $outputRoot 'stop-safety.stderr.txt'
    $p = Start-Process -FilePath (Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe') -ArgumentList '-NoLogo','-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30' -WorkingDirectory $repo -WindowStyle Hidden -RedirectStandardOutput $outPath -RedirectStandardError $errPath -PassThru
    $ownedHandle = [IntPtr]::Zero
    $handleOpened = $false
    $handleOpenError = $null
    $initialExit = $null
    try {
        try {
            $ownedHandle = $p.Handle
            $handleOpened = ($ownedHandle -ne [IntPtr]::Zero)
            if ($handleOpened) { $initialExit = Read-OwnedExitCode $ownedHandle }
        } catch {
            $handleOpenError = $_.Exception.Message
        }
        $testPid = $p.Id
        $expected = Read-ProcessCreationStamp $testPid
        $wrong = if ($expected) { Stop-OwnedTree $testPid ($expected + '-wrong') } else { [ordered]@{ stopped = $false; reason = 'creation-stamp-unavailable; refused' } }
        $afterWrong = if ($handleOpened) { Read-OwnedExitCode $ownedHandle } else { $null }
        $correct = if ($expected) { Stop-OwnedTree $testPid $expected } else { [ordered]@{ stopped = $false; reason = 'creation-stamp-unavailable; refused' } }
        [void]$p.WaitForExit(5000)
        $afterCorrect = if ($handleOpened) { Read-OwnedExitCode $ownedHandle } else { $null }
        $fallbackTerminate = $false
        if ($handleOpened -and $afterCorrect -and $afterCorrect.available -and $afterCorrect.still_active) {
            $fallbackTerminate = $script:ExitCodeApiType::TerminateProcess($ownedHandle, [uint32]1)
            [void]$p.WaitForExit(5000)
            $afterCorrect = Read-OwnedExitCode $ownedHandle
        }
        $wrongRejected = [bool]($wrong -and -not $wrong.stopped -and $wrong.reason -match 'creation-stamp-mismatch')
        $liveAfterWrong = [bool]($afterWrong -and $afterWrong.available -and $afterWrong.still_active)
        $correctStopped = [bool]($correct -and $correct.stopped)
        $exitConfirmed = [bool]($afterCorrect -and $afterCorrect.available -and -not $afterCorrect.still_active)
        $status = if ($handleOpened -and $initialExit -and $initialExit.still_active -and $wrongRejected -and $liveAfterWrong -and $correctStopped -and $exitConfirmed) { 'Verified' } else { 'Unverified' }
        return [ordered]@{
            status = $status
            pid = $testPid
            expected_creation_stamp = $expected
            wrapper_handle_opened = $handleOpened
            wrapper_handle_open_error = $handleOpenError
            initial_exit = $initialExit
            wrong_stamp = ($expected + '-wrong')
            wrong_result = $wrong
            after_wrong_exit = $afterWrong
            wrong_stamp_rejected = $wrongRejected
            live_after_wrong_stamp = $liveAfterWrong
            correct_result = $correct
            after_correct_exit = $afterCorrect
            correct_stamp_stopped = $correctStopped
            exit_confirmed_on_original_handle = $exitConfirmed
            fallback_terminate_on_original_handle = $fallbackTerminate
        }
    }
    finally {
        if ($ownedHandle -ne [IntPtr]::Zero) { [void]$script:ExitCodeApiType::CloseHandle($ownedHandle) }
    }
}

function New-Entry([string]$entry, [string]$status, [string]$method, [string]$reason, [object]$observations) {
    return [ordered]@{
        entry = $entry
        status = $status
        method = $method
        reason = $reason
        observations = $observations
    }
}

function New-Profile {
    $oldConfig = $env:ENVBOX_CONFIG_ROOT
    $env:ENVBOX_CONFIG_ROOT = $configRoot
    try {
        $p = Start-Process -FilePath $envbox -ArgumentList 'profile add --name "Real app matrix" --locale en-US --ui-language en-US --region US --tz-windows "Pacific Standard Time" --tz-iana America/Los_Angeles --env ENVBOX_MATRIX=profile --env LANG=en_US.UTF-8' -WorkingDirectory $repo -WindowStyle Hidden -RedirectStandardOutput (Join-Path $outputRoot 'profile.stdout.txt') -RedirectStandardError (Join-Path $outputRoot 'profile.stderr.txt') -PassThru
        if (-not $p.WaitForExit(30000)) { throw 'profile add timed out' }
        $p.Refresh()
        $id = (Get-Content -LiteralPath (Join-Path $outputRoot 'profile.stdout.txt') -Raw).Trim()
        if ($id -notmatch '^[0-9a-fA-F-]{36}$') { throw "profile add returned invalid id: $id" }
        return $id
    } finally {
        if ($null -eq $oldConfig) { Remove-Item Env:ENVBOX_CONFIG_ROOT -ErrorAction SilentlyContinue } else { $env:ENVBOX_CONFIG_ROOT = $oldConfig }
    }
}

function Invoke-HostProbe([string]$name) {
    # The whole matrix worker was created by the outer Win32_Process.Create
    # call. Do not issue a nested WMI Create here: the provider can hold the
    # worker callback while waiting for its child and deadlock the control.
    # This direct probe is the independent uninjected host baseline; the
    # worker itself is the real WMI-entry observation recorded below.
    $outPath = Join-Path $outputRoot "$name-host.txt"
    $p = Start-Process -FilePath $probe -ArgumentList '--child' -WorkingDirectory $repo -WindowStyle Hidden -RedirectStandardOutput $outPath -RedirectStandardError ($outPath + '.err') -PassThru
    $wait = $p.WaitForExit(30000)
    $p.Refresh()
    $text = if (Test-Path -LiteralPath $outPath) { Get-Content -LiteralPath $outPath -Raw } else { '' }
    Preserve-Artifact $outPath "$name-host.txt"
    Preserve-Artifact ($outPath + '.err') "$name-host.txt.err"
    return [ordered]@{ pid = $p.Id; waited = $wait; exit_code = if ($p.HasExited) { $p.ExitCode } else { $null }; output = $text; output_path = $outPath; runtime_marker_count = ([regex]::Matches($text, 'EnvBox Runtime Loaded')).Count }
}

function Invoke-ShellExecuteProbe([string]$name) {
    $outPath = Join-Path $outputRoot "$name-shell.txt"
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $probe
    $psi.Arguments = (Quote-ProcessArgument '--as-user-child-output') + ' ' + (Quote-ProcessArgument $outPath)
    $psi.UseShellExecute = $true
    $psi.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $p = [Diagnostics.Process]::Start($psi)
    $wait = $p.WaitForExit(30000)
    $p.Refresh()
    $text = if (Test-Path -LiteralPath $outPath) { Get-Content -LiteralPath $outPath -Raw } else { '' }
    Preserve-Artifact $outPath "$name-shell.txt"
    return [ordered]@{ pid = $p.Id; waited = $wait; exit_code = if ($p.HasExited) { $p.ExitCode } else { $null }; output = $text; output_path = $outPath; runtime_marker_count = ([regex]::Matches($text, 'EnvBox Runtime Loaded')).Count }
}

function Find-Browser {
    foreach ($path in @(
        'C:\Program Files\Google\Chrome\Application\chrome.exe',
        'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe',
        'C:\Program Files\Microsoft\Edge\Application\msedge.exe'
    )) {
        if (Test-Path -LiteralPath $path -PathType Leaf) { return $path }
    }
    return $null
}

$profileId = New-Profile
Add-Content -LiteralPath $tracePath -Value "profile=$profileId"
$hostControl = Invoke-HostProbe 'host-control'
Add-Content -LiteralPath $tracePath -Value "host-control pid=$($hostControl.pid)"
$stopSafety = Invoke-StopOwnedTreeSafetyCheck
Add-Content -LiteralPath $tracePath -Value "stop-safety status=$($stopSafety.status) pid=$($stopSafety.pid)"
$entries = @()

$probeRun = Invoke-EnvBox 'createprocess-probe' $runtime64 @('run','--profile',$profileId,'--',$probe,'--spawn-child','--spawn-as-user-child') 60000
Add-Content -LiteralPath $tracePath -Value "probe exit=$($probeRun.exit_code) pid=$($probeRun.target_pid)"
$probeMarkerCount = ([regex]::Matches($probeRun.stdout, 'EnvBox Runtime Loaded')).Count
$probeProfileCount = ([regex]::Matches($probeRun.stdout, 'ENVBOX_MATRIX:\r?\nprofile')).Count
$probeAsUserStatus = (($probeRun.stdout -split "`r?`n") | Where-Object { $_ -match '^Status:' } | Select-Object -First 1)
$probeRuntimeStarted = $probeRun.stderr -match 'Runtime \+ core hooks active'
$probeCompleted = (-not $probeRun.timed_out -and $probeRun.wrapper_exit_observed -and $probeRun.exit_code -eq 0)
$probeStartupVerified = ($probeMarkerCount -ge 3 -and $probeProfileCount -ge 3 -and $probeAsUserStatus -match 'Status:\s*succeeded' -and $probeRuntimeStarted)
$probeStatus = if ($probeCompleted -and $probeStartupVerified) {
    'Verified'
} elseif ($probeStartupVerified -or $probeMarkerCount -gt 0 -or $probeProfileCount -gt 0) {
    'Partial'
} else {
    'Unverified'
}
$entries += New-Entry 'CreateProcess + CreateProcessAsUserW' $probeStatus 'envbox run -> Detours CreateProcessW; envbox-probe child + AsUser child' 'Startup observations and completion are reported separately. Verified requires the wrapper to finish with exit code 0; an unavailable or non-zero wrapper exit code leaves the row Partial/Unverified even when the target printed Runtime markers.' ([ordered]@{ result = $probeRun; startup_observation = if ($probeStartupVerified) { 'Verified' } else { 'Partial/Unverified' }; completion_observation = [ordered]@{ wrapper_exit_observed = $probeRun.wrapper_exit_observed; timed_out = $probeRun.timed_out; exit_code = $probeRun.exit_code }; marker_count = $probeMarkerCount; profile_env_count = $probeProfileCount; runtime_start_line = $probeRuntimeStarted; as_user_status = $probeAsUserStatus })
Add-Content -LiteralPath $tracePath -Value 'createprocess done'

$cmdRun = Invoke-EnvBox 'console-cmd' $runtime64 @('run','--profile',$profileId,'--','cmd.exe','/d','/c','echo ENVBOX_MATRIX=%ENVBOX_MATRIX%') 30000
$cmdRuntimeStarted = $cmdRun.stderr -match 'Runtime \+ core hooks active'
$cmdStartupVerified = ($cmdRun.stdout -match 'ENVBOX_MATRIX=profile' -and $cmdRuntimeStarted)
$cmdCompleted = (-not $cmdRun.timed_out -and $cmdRun.wrapper_exit_observed -and $cmdRun.exit_code -eq 0)
$cmdStatus = if ($cmdStartupVerified -and $cmdCompleted) { 'Verified' } elseif ($cmdStartupVerified) { 'Partial' } else { 'Unverified' }
$entries += New-Entry 'console cmd.exe' $cmdStatus 'envbox run -> direct CreateProcessW of cmd.exe' 'Startup output and completion are separate facts. Verified requires wrapper exit code 0; an unavailable exit code leaves the row Partial even when cmd printed the Profile environment.' ([ordered]@{ result = $cmdRun; startup_observation = if ($cmdStartupVerified) { 'Verified' } else { 'Unverified' }; completion_observation = [ordered]@{ wrapper_exit_observed = $cmdRun.wrapper_exit_observed; timed_out = $cmdRun.timed_out; exit_code = $cmdRun.exit_code }; marker_count = ([regex]::Matches($cmdRun.stdout, 'EnvBox Runtime Loaded')).Count; runtime_start_line = $cmdRuntimeStarted })
Add-Content -LiteralPath $tracePath -Value 'cmd done'

$psRun = Invoke-EnvBox 'console-powershell' $runtime64 @('run','--profile',$profileId,'--','powershell.exe','-NoLogo','-NoProfile','-NonInteractive','-Command','Write-Output $env:ENVBOX_MATRIX') 30000
$psRuntimeStarted = $psRun.stderr -match 'Runtime \+ core hooks active'
$psStartupVerified = ($psRun.stdout -match 'profile' -and $psRuntimeStarted)
$psCompleted = (-not $psRun.timed_out -and $psRun.wrapper_exit_observed -and $psRun.exit_code -eq 0)
$psStatus = if ($psStartupVerified -and $psCompleted) { 'Verified' } elseif ($psStartupVerified) { 'Partial' } elseif ($psRun.stderr -match 'Unsupported|unsupported|TLS|runtime') { 'Unsupported' } else { 'Unverified' }
$entries += New-Entry 'console PowerShell' $psStatus 'envbox run -> direct CreateProcessW of powershell.exe' 'PowerShell startup and completion are separate facts. Verified requires wrapper exit code 0; the result is recorded from the real invocation and is not generalized to other PowerShell versions.' ([ordered]@{ result = $psRun; startup_observation = if ($psStartupVerified) { 'Verified' } else { 'Unverified' }; completion_observation = [ordered]@{ wrapper_exit_observed = $psRun.wrapper_exit_observed; timed_out = $psRun.timed_out; exit_code = $psRun.exit_code }; marker_count = ([regex]::Matches($psRun.stdout, 'EnvBox Runtime Loaded')).Count; runtime_start_line = $psRuntimeStarted })
Add-Content -LiteralPath $tracePath -Value 'powershell done'

$entries += New-Entry 'ShellExecute' 'Unsupported' 'ShellExecute-compatible ProcessStartInfo.UseShellExecute=true' 'A real hidden shell launch produced no Runtime marker and no Profile environment. The product does not claim global ShellExecute interception; no existing user application was opened.' (Invoke-ShellExecuteProbe 'shell-execute')
Add-Content -LiteralPath $tracePath -Value 'shell done'
$entries += New-Entry 'WMI Win32_Process.Create' 'Unsupported' 'outer Win32_Process.Create created the fresh matrix worker' 'The actual matrix worker was created through WMI and its uninjected probe had no Runtime marker. A nested WMI Create is intentionally not attempted because the provider callback can deadlock its worker; this row records the real WMI entry boundary without claiming target injection.' (Invoke-HostProbe 'wmi')
Add-Content -LiteralPath $tracePath -Value 'wmi done'

$browser = Find-Browser
$browserCleanup = [ordered]@{ attempted = $false; target_pid = $null; target_exists_after_stop = $null; remaining_tree = @() }
$hostBrowserControl = $null
if ($browser) {
    $browserName = [IO.Path]::GetFileNameWithoutExtension($browser)
    $hostBrowserProfile = Join-Path $root 'chrome-host-profile'
    New-Item -ItemType Directory -Force -Path $hostBrowserProfile | Out-Null
    $hostBrowserControl = Compact-RunResult (Invoke-HostBrowser $browser $hostBrowserProfile)
    Add-Content -LiteralPath $tracePath -Value "host-browser target=$($hostBrowserControl.target_pid) exit=$($hostBrowserControl.exit_code) timed_out=$($hostBrowserControl.timed_out)"
    $browserRun = Invoke-EnvBox 'browser-root' $runtime64 @('run','--profile',$profileId,'--',$browser,'--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--user-data-dir',$browserProfile,'--remote-debugging-port=0','about:blank') 15000 -CaptureTree
    if ($browserRun.target_pid) {
        $browserCleanup.attempted = $true
        $browserCleanup.target_pid = [int]$browserRun.target_pid
        $browserCleanup.stop = Stop-OwnedTree ([int]$browserRun.target_pid) $browserRun.target_creation_stamp
        $browserCleanup.target_creation_stamp_after = Read-ProcessCreationStamp ([int]$browserRun.target_pid)
        $browserCleanup.target_exists_after_stop = ($null -ne $browserCleanup.target_creation_stamp_after -and $browserCleanup.target_creation_stamp_after -eq $browserRun.target_creation_stamp)
        $browserCleanup.remaining_tree = @(Read-ProcessTree ([int]$browserRun.target_pid))
    }
    $rendererRows = @($browserRun.runtime_tree | Where-Object { $_.command_line -match '--type=renderer' })
    $rootLoaded = @($browserRun.runtime_tree | Where-Object { $_.pid -eq $browserRun.target_pid -and $_.runtime_count -gt 0 }).Count -gt 0
    $rendererLoaded = @($rendererRows | Where-Object { $_.runtime_count -gt 0 }).Count -gt 0
    $browserStatus = if ($rootLoaded) { 'Partial' } else { 'Unverified' }
    $rendererStatus = if ($rendererLoaded) { 'Observed' } else { 'NotObserved' }
    $entries += New-Entry 'Chromium/Edge browser root + renderer' $browserStatus 'real installed browser, hidden headless, isolated temporary user-data-dir plus uninjected host control' 'The host control and Aura launch use separate temporary profiles and the same hidden headless arguments. A short-lived target with no Runtime tree and no verified exit result is Unverified; it is not classified Unsupported from a missing sample. Renderer coverage is explicitly capped at Partial/Unsupported and never Verified; no --no-sandbox switch was used.' ([ordered]@{ browser = $browser; version = (Get-Item -LiteralPath $browser).VersionInfo.FileVersion; host_control = $hostBrowserControl; result = $browserRun; renderer_processes = $rendererRows; root_runtime_loaded = $rootLoaded; renderer_runtime_loaded = $rendererLoaded; renderer_coverage = $rendererStatus; single_instance_transfer = 'Unverified (no existing user singleton was touched)' })
    Add-Content -LiteralPath $tracePath -Value "browser done status=$browserStatus"
} else {
    $entries += New-Entry 'Chromium/Edge browser root + renderer' 'Unverified' 'installed-browser discovery' 'No supported installed browser executable was found.' ([ordered]@{ candidates = @('Chrome','Edge') })
}

$entries += New-Entry 'WithToken' 'Unverified' 'no product entry adapter exercised' 'The current workspace has a real CreateProcessAsUserW probe, but no safe standalone WithToken fixture. This row remains Unverified rather than inferring support from AsUser.' ([ordered]@{ })
$entries += New-Entry 'Native NtCreateUserProcess' 'Unverified' 'no product entry adapter exercised' 'No Native API fixture was run on the host. The matrix intentionally does not substitute CreateProcessW or WMI for Native launch.' ([ordered]@{ })
$entries += New-Entry 'Packaged/AUMID' 'Unverified' 'no safe hidden packaged candidate' 'Installed packaged applications may open visible UI or reuse a singleton. No existing app was activated; the cold-start and single-instance result therefore remains Unverified.' ([ordered]@{ installed_packages = 'not-enumerated; activation was intentionally skipped' })
Add-Content -LiteralPath $tracePath -Value 'entry rows done'

# Preserve full stdout/stderr as target artifacts, but keep the JSON summary
# bounded.  A probe contains a complete environment block and should never
# make the acceptance handoff allocate unbounded memory during serialization.
foreach ($entry in $entries) {
    $observed = $entry.observations
    if ($observed -and $observed.result) { $observed.result = Compact-RunResult $observed.result }
    if ($observed -and $observed.output) {
        $text = [string]$observed.output
        $observed.output_length = $text.Length
        $observed.output_excerpt = if ($text.Length -gt 4096) { $text.Substring(0, 4096) + "`n[truncated; raw artifact is in target/real-app-matrix]" } else { $text }
        $observed.output = $null
    }
}
if ($hostControl) {
    $hostText = [string]$hostControl.output
    $compactHost = [ordered]@{}
    foreach ($key in $hostControl.Keys) {
        if ($key -eq 'output') { continue }
        $compactHost[$key] = $hostControl[$key]
    }
    $compactHost.output_length = $hostText.Length
    $compactHost.output_excerpt = if ($hostText.Length -gt 4096) { $hostText.Substring(0, 4096) + "`n[truncated; raw artifact is in target/real-app-matrix]" } else { $hostText }
    $hostControl = $compactHost
}

$sourceHash = (Get-FileHash -LiteralPath $runtime64 -Algorithm SHA256).Hash
$sourceHash32 = (Get-FileHash -LiteralPath $runtime32 -Algorithm SHA256).Hash
$hostFacts = [ordered]@{
    worker_pid = $PID
    worker_runtime_modules = $workerModules.Count
    os = (Get-CimInstance Win32_OperatingSystem | Select-Object -First 1 Version,BuildNumber,Caption)
    powershell = $PSVersionTable.PSVersion.ToString()
    runtime64 = [ordered]@{ path = $runtime64; sha256 = $sourceHash }
    runtime32 = [ordered]@{ path = $runtime32; sha256 = $sourceHash32 }
    envbox = $envbox
    probe = $probe
    browser_probe = $browserProbe
    config_root = $configRoot
    temporary_profile = $browserProfile
    host_control = $hostControl
    host_browser_control = $hostBrowserControl
    stop_owned_tree_safety = $stopSafety
    browser_cleanup = $browserCleanup
}
Add-Content -LiteralPath $tracePath -Value 'host facts done'

$cleanup = [ordered]@{
    requested_root = $root
    resolved_root = $null
    intended_parent = [IO.Path]::GetFullPath(([IO.Path]::GetTempPath()))
    contained_by_intended_parent = $false
    root_was_reparse_point = $false
    processes_using_root = @()
    removed = $false
    residual_paths = @()
    error = $null
}
try {
    $item = Get-Item -LiteralPath $root -Force -ErrorAction Stop
    $resolvedRoot = [IO.Path]::GetFullPath($item.FullName)
    $resolvedParent = [IO.Path]::GetFullPath(([IO.Path]::GetTempPath()))
    $cleanup.resolved_root = $resolvedRoot
    $cleanup.root_was_reparse_point = (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)
    $parentPrefix = $resolvedParent.TrimEnd('\') + '\'
    if (-not $resolvedRoot.StartsWith($parentPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing cleanup outside intended temp parent: $resolvedRoot"
    }
    if ($cleanup.root_was_reparse_point) {
        throw "refusing cleanup of reparse-point root: $resolvedRoot"
    }
    $cleanup.contained_by_intended_parent = $true
    $cleanup.processes_using_root = @(Read-ProcessesUsingPath $resolvedRoot)
    if ($cleanup.processes_using_root.Count -ne 0) {
        throw "refusing cleanup while owned or unknown process command lines reference $resolvedRoot"
    }
    Remove-Item -LiteralPath $resolvedRoot -Recurse -Force -ErrorAction Stop
    $cleanup.removed = -not (Test-Path -LiteralPath $resolvedRoot)
    if (-not $cleanup.removed) { throw "cleanup returned without removing $resolvedRoot" }
} catch {
    $cleanup.error = $_.Exception.Message
    if ($cleanup.resolved_root -and (Test-Path -LiteralPath $cleanup.resolved_root)) {
        $cleanup.residual_paths = @(Get-ChildItem -LiteralPath $cleanup.resolved_root -Force -Recurse -ErrorAction SilentlyContinue | ForEach-Object FullName)
    }
}
Add-Content -LiteralPath $tracePath -Value "cleanup done error=$($cleanup.error) removed=$($cleanup.removed)"

$summary = [ordered]@{
    schema = 1
    generated_utc = [DateTime]::UtcNow.ToString('o')
    host_fresh_wmi = $true
    harness_status = if ($cleanup.error) { 'cleanup-failed' } else { 'completed' }
    matrix_status = 'partial-validation; statuses are per-entry and unsupported/unverified are retained'
    exit_code = if ($cleanup.error) { 2 } else { 0 }
    host = $hostFacts
    entries = $entries
    cleanup = $cleanup
}
Save-Json $summary $resultPath
Save-Json $summary (Join-Path $logDir 'summary.json')
Write-Output ($summary | ConvertTo-Json -Depth 12)
