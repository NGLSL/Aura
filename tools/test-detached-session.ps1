param(
    [string]$Cli = '',
    [string]$RuntimeDirectory = '',
    [string]$ConfigRoot = '',
    [string]$Profile = 'detached-session',
    [string]$PowerShell = (Get-Command pwsh -ErrorAction Stop).Source,
    [ValidateSet('Normal', 'Forced', 'Both', 'ConsoleClose', 'All')][string]$Scenario = 'Both',
    [switch]$IncludeDeveloperTools,
    [int]$TimeoutSeconds = 45
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (!$Cli) { $Cli = Join-Path $repo 'target/debug/envbox.exe' }
$Cli = (Resolve-Path -LiteralPath $Cli).Path
$PowerShell = (Resolve-Path -LiteralPath $PowerShell).Path
if (@([Diagnostics.Process]::GetCurrentProcess().Modules | Where-Object ModuleName -Like 'envbox-runtime*').Count) {
    throw 'Run the regression controller from a fresh, uninjected PowerShell process.'
}
$evidence = Join-Path ([IO.Path]::GetTempPath()) ('aura-detached-session-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $evidence | Out-Null
if ($ConfigRoot) {
    Copy-Item -LiteralPath (Join-Path $ConfigRoot 'profiles.toml') -Destination $evidence
} else {
    $profileInfo = [Diagnostics.ProcessStartInfo]::new()
    $profileInfo.FileName = $Cli; $profileInfo.UseShellExecute = $false; $profileInfo.CreateNoWindow = $true
    $profileInfo.RedirectStandardOutput = $true; $profileInfo.RedirectStandardError = $true
    foreach ($name in @($profileInfo.Environment.Keys | Where-Object { $_ -like 'ENVBOX_*' })) { $profileInfo.Environment.Remove($name) | Out-Null }
    $profileInfo.Environment['ENVBOX_CONFIG_ROOT'] = $evidence
    foreach ($arg in @('profile', 'add', '--name', $Profile, '--locale', 'en-US', '--ui-language', 'en-US', '--region', 'US', '--tz-windows', 'UTC', '--tz-iana', 'Etc/UTC', '--dns-mode', 'host')) { $profileInfo.ArgumentList.Add($arg) }
    $profileProcess = [Diagnostics.Process]::Start($profileInfo)
    $profileOut = $profileProcess.StandardOutput.ReadToEndAsync(); $profileErr = $profileProcess.StandardError.ReadToEndAsync()
    if (!$profileProcess.WaitForExit($TimeoutSeconds * 1000)) { $profileProcess.Kill(); $profileProcess.WaitForExit(); throw 'Fixture Profile creation timed out' }
    $profileOut.Result | Set-Content -LiteralPath (Join-Path $evidence 'profile.stdout.log')
    $profileErr.Result | Set-Content -LiteralPath (Join-Path $evidence 'profile.stderr.log')
    $profileCode = $profileProcess.ExitCode; $profileProcess.Dispose()
    if ($profileCode -ne 0) { throw "Fixture Profile creation failed. Evidence: $evidence" }
    $expectedProfile = $profileOut.Result.Trim()
}
$targets = @([pscustomobject]@{ File = (Join-Path $env:WINDIR 'System32/cmd.exe'); Arguments = @('/d', '/c', 'ver') })
if ($IncludeDeveloperTools) {
    foreach ($name in @('git', 'cargo', 'rustc')) {
        $targets += [pscustomobject]@{ File = (Get-Command $name -ErrorAction Stop).Source; Arguments = @('--version') }
    }
}
$targets | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $evidence 'targets.json')

# Never connect to the broker: an empty protocol connection could change the fixture.
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DetachedPipeProbe {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool WaitNamedPipe(string name, uint timeout);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
'@
function Test-BrokerPipe([string]$Name) {
    if (!$Name.StartsWith('\\.\pipe\')) { $Name = '\\.\pipe\' + $Name }
    if ([DetachedPipeProbe]::WaitNamedPipe($Name, 1)) { return $true }
    # ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND mean the server has gone.
    return [Runtime.InteropServices.Marshal]::GetLastWin32Error() -notin @(2, 3)
}
function Wait-File([string]$Path, [Diagnostics.Process]$Owner) {
    $limit = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while (!(Test-Path -LiteralPath $Path)) {
        if ($Owner -and $Owner.HasExited) { throw "Fixture exited before writing $Path" }
        if ([DateTime]::UtcNow -gt $limit) { throw "Timed out waiting for $Path" }
        Start-Sleep -Milliseconds 50
    }
}

$childScript = Join-Path $evidence 'child.ps1'
@'
param([string]$Directory, [string]$Shell, [int]$TimeoutSeconds)
$ErrorActionPreference = 'Stop'
$snapshot = [ordered]@{
    Pid = $PID; Pipe = $env:ENVBOX_IPC_PIPE; Profile = $env:ENVBOX_PROFILE_ID
    RuntimeLoaded = $env:ENVBOX_RUNTIME_LOADED; Locale = $env:ENVBOX_LOCALE_NAME
    Timezone = [TimeZoneInfo]::Local.Id
}
$snapshot | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Directory 'ready.tmp')
Move-Item -LiteralPath (Join-Path $Directory 'ready.tmp') -Destination (Join-Path $Directory 'ready.json')
$limit = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
while (!(Test-Path -LiteralPath (Join-Path $Directory 'continue'))) {
    if ([DateTime]::UtcNow -gt $limit) { exit 2 }
    Start-Sleep -Milliseconds 25
}
$results = @()
$targets = @(Get-Content -Raw -LiteralPath (Join-Path $Directory 'targets.json') | ConvertFrom-Json)
# This descendant checks the actual hooked timezone API, not only ENVBOX variables.
$targets += [pscustomobject]@{ File = $Shell; Arguments = @('-NoProfile', '-File', (Join-Path $Directory 'grandchild.ps1')) }
foreach ($target in $targets) {
    $process = $null
    try {
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $target.File
        foreach ($arg in $target.Arguments) { $info.ArgumentList.Add([string]$arg) }
        $info.UseShellExecute = $false; $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
        $process = [Diagnostics.Process]::Start($info)
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(10000)) { $process.Kill(); $process.WaitForExit(); throw 'Descendant timed out' }
        $results += [pscustomobject]@{ File = $target.File; ExitCode = $process.ExitCode; Output = $stdout.Result; Error = $stderr.Result }
    } catch {
        $results += [pscustomobject]@{ File = $target.File; ExitCode = -1; Output = ''; Error = $_.Exception.Message }
    } finally { if ($process) { $process.Dispose() } }
}
$results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Directory 'results.tmp')
Move-Item -LiteralPath (Join-Path $Directory 'results.tmp') -Destination (Join-Path $Directory 'results.json')
if (@($results | Where-Object ExitCode -NE 0).Count) { exit 1 }
'@ | Set-Content -LiteralPath $childScript

$rootScript = Join-Path $evidence 'root.ps1'
@'
param([string]$Directory, [string]$Shell, [int]$TimeoutSeconds, [string]$Mode)
$ErrorActionPreference = 'Stop'
$PID | Set-Content -LiteralPath (Join-Path $Directory 'owned-root.pid')
$info = [Diagnostics.ProcessStartInfo]::new()
$info.FileName = $Shell
foreach ($arg in @('-NoProfile', '-File', (Join-Path $Directory 'child.ps1'), '-Directory', $Directory, '-Shell', $Shell, '-TimeoutSeconds', "$TimeoutSeconds")) { $info.ArgumentList.Add($arg) }
$info.UseShellExecute = $false; $info.CreateNoWindow = $true
$child = [Diagnostics.Process]::Start($info)
$child.Id | Set-Content -LiteralPath (Join-Path $Directory 'owned-child.pid')
$limit = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
while (!(Test-Path -LiteralPath (Join-Path $Directory 'ready.json'))) {
    if ($child.HasExited -or [DateTime]::UtcNow -gt $limit) { throw 'Retained child failed to become ready' }
    Start-Sleep -Milliseconds 25
}
if ($Mode -eq 'Forced') {
    while (!(Test-Path -LiteralPath (Join-Path $Directory 'root-exit'))) { Start-Sleep -Milliseconds 25 }
}
'@ | Set-Content -LiteralPath $rootScript

$consoleScript = Join-Path $evidence 'console.ps1'
@'
param([string]$Directory, [string]$Config, [string]$Cli, [string]$Runtime, [string]$Profile, [string]$Shell, [string]$RootScript, [int]$TimeoutSeconds)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class FixtureConsole {
    [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
"@
$window = [FixtureConsole]::GetConsoleWindow()
[uint32]$owner = 0
[FixtureConsole]::GetWindowThreadProcessId($window, [ref]$owner) | Out-Null
if ($window -eq [IntPtr]::Zero -or !$owner) { throw 'Hidden fixture console was not created' }
[ordered]@{ WrapperPid = $PID; Window = $window.ToInt64(); OwnerPid = $owner; OwnerStarted = (Get-Process -Id $owner).StartTime.ToUniversalTime().Ticks } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Directory 'console.tmp')
Move-Item -LiteralPath (Join-Path $Directory 'console.tmp') -Destination (Join-Path $Directory 'console.json')
Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
$env:ENVBOX_CONFIG_ROOT = $Config
if ($Runtime) { $env:ENVBOX_RUNTIME_DLL = $Runtime }
& $Cli run --profile $Profile --audit -- $Shell -NoProfile -File $RootScript -Directory $Directory -Shell $Shell -TimeoutSeconds $TimeoutSeconds -Mode Forced 1> (Join-Path $Directory 'cli.stdout.log') 2> (Join-Path $Directory 'cli.stderr.log')
exit $LASTEXITCODE
'@ | Set-Content -LiteralPath $consoleScript

$modes = switch ($Scenario) { 'Both' { @('Normal', 'Forced') }; 'All' { @('Normal', 'Forced', 'ConsoleClose') }; default { @($Scenario) } }
$failures = @()
Write-Host "Evidence: $evidence"
foreach ($mode in $modes) {
    $directory = Join-Path $evidence $mode
    New-Item -ItemType Directory -Path $directory | Out-Null
    foreach ($file in @('child.ps1', 'targets.json')) { Copy-Item -LiteralPath (Join-Path $evidence $file) -Destination $directory }
    @'
[ordered]@{ RuntimeLoaded = $env:ENVBOX_RUNTIME_LOADED; Profile = $env:ENVBOX_PROFILE_ID; Locale = $env:ENVBOX_LOCALE_NAME; Timezone = [TimeZoneInfo]::Local.Id } | ConvertTo-Json -Compress
'@ | Set-Content -LiteralPath (Join-Path $directory 'grandchild.ps1')
    $hostProcess = $null; $child = $null; $root = $null; $stdout = $null; $stderr = $null
    try {
        if ($mode -eq 'ConsoleClose') {
            $runtime = if ($RuntimeDirectory) { (Resolve-Path -LiteralPath (Join-Path $RuntimeDirectory 'envbox-runtime64.dll')).Path } else { '' }
            # Start-Process creates a separate hidden console; never attach to or close the user's console.
            $wrapperArguments = @('-NoProfile', '-File', $consoleScript, '-Directory', $directory, '-Config', $evidence, '-Cli', $Cli, '-Profile', $Profile, '-Shell', $PowerShell, '-RootScript', $rootScript, '-TimeoutSeconds', "$TimeoutSeconds")
            if ($runtime) { $wrapperArguments += @('-Runtime', $runtime) }
            $quotedArguments = ($wrapperArguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
            $hostProcess = Start-Process -FilePath $PowerShell -ArgumentList $quotedArguments -WindowStyle Hidden -PassThru
        } else {
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $Cli; $info.UseShellExecute = $false; $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true; $info.RedirectStandardError = $true
        foreach ($name in @($info.Environment.Keys | Where-Object { $_ -like 'ENVBOX_*' })) { $info.Environment.Remove($name) | Out-Null }
        $info.Environment['ENVBOX_CONFIG_ROOT'] = $evidence
        if ($RuntimeDirectory) { $info.Environment['ENVBOX_RUNTIME_DLL'] = (Resolve-Path -LiteralPath (Join-Path $RuntimeDirectory 'envbox-runtime64.dll')).Path }
        foreach ($arg in @('run', '--profile', $Profile, '--audit', '--', $PowerShell, '-NoProfile', '-File', $rootScript, '-Directory', $directory, '-Shell', $PowerShell, '-TimeoutSeconds', "$TimeoutSeconds", '-Mode', $mode)) { $info.ArgumentList.Add($arg) }
        $hostProcess = [Diagnostics.Process]::Start($info)
        $stdout = $hostProcess.StandardOutput.ReadToEndAsync(); $stderr = $hostProcess.StandardError.ReadToEndAsync()
        }
        Wait-File (Join-Path $directory 'ready.json') $hostProcess
        $ready = Get-Content -Raw -LiteralPath (Join-Path $directory 'ready.json') | ConvertFrom-Json
        $child = [Diagnostics.Process]::GetProcessById($ready.Pid)
        if ($mode -ne 'Normal') { $root = [Diagnostics.Process]::GetProcessById([int](Get-Content -LiteralPath (Join-Path $directory 'owned-root.pid'))) }
        if ($ready.RuntimeLoaded -ne '1' -or !$ready.Profile -or !$ready.Pipe) { throw 'Retained child did not inherit an injected Profile session' }
        if (!$ConfigRoot -and ($ready.Profile -ne $expectedProfile -or $ready.Locale -ne 'en-US' -or $ready.Timezone -ne 'UTC')) { throw 'Retained child differs from the fixture Profile' }
        if ($mode -eq 'ConsoleClose') {
            $console = Get-Content -Raw -LiteralPath (Join-Path $directory 'console.json') | ConvertFrom-Json
            if ($console.WrapperPid -ne $hostProcess.Id) { throw 'Console does not belong to the fixture wrapper' }
            [uint32]$owner = 0
            [DetachedPipeProbe]::GetWindowThreadProcessId([IntPtr]$console.Window, [ref]$owner) | Out-Null
            $ownerProcess = Get-Process -Id $owner -ErrorAction Stop
            if ($owner -ne $console.OwnerPid -or $ownerProcess.StartTime.ToUniversalTime().Ticks -ne $console.OwnerStarted) { throw 'Fixture console ownership changed; refusing to close it' }
            if (![DetachedPipeProbe]::PostMessage([IntPtr]$console.Window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Could not close the fixture console' }
            if (!$hostProcess.WaitForExit($TimeoutSeconds * 1000) -or !$root.WaitForExit($TimeoutSeconds * 1000)) { throw 'Fixture console processes did not exit after WM_CLOSE' }
            if (!(Test-BrokerPipe $ready.Pipe)) { throw 'Broker disappeared when its original console closed' }
        } elseif ($mode -eq 'Forced') {
            # Kill only the CLI spawned by this controller. The root exits normally next.
            $hostProcess.Kill(); $hostProcess.WaitForExit()
            New-Item -ItemType File -Path (Join-Path $directory 'root-exit') | Out-Null
            if (!$root.WaitForExit($TimeoutSeconds * 1000)) { throw 'Root did not exit after its launcher stopped' }
        } elseif (!$hostProcess.WaitForExit($TimeoutSeconds * 1000)) { throw 'CLI did not exit after its root exited normally' }
        New-Item -ItemType File -Path (Join-Path $directory 'continue') | Out-Null
        Wait-File (Join-Path $directory 'results.json') $null
        $results = @(Get-Content -Raw -LiteralPath (Join-Path $directory 'results.json') | ConvertFrom-Json)
        $results | Format-Table File, ExitCode, Error -AutoSize | Out-Host
        if (@($results | Where-Object ExitCode -NE 0).Count) { throw 'Descendant creation failed after launcher exit' }
        $grandchild = $results[-1].Output | ConvertFrom-Json
        if ($grandchild.RuntimeLoaded -ne '1' -or $grandchild.Profile -ne $ready.Profile -or $grandchild.Locale -ne $ready.Locale -or $grandchild.Timezone -ne $ready.Timezone) { throw 'Descendant did not retain the inherited Profile and timezone API view' }
        if (!$child.WaitForExit($TimeoutSeconds * 1000)) { throw 'Retained child did not exit' }
        $limit = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while (Test-BrokerPipe $ready.Pipe) {
            if ([DateTime]::UtcNow -gt $limit) { throw 'Broker remained alive after the final fixture process exited' }
            Start-Sleep -Milliseconds 100
        }
        $auditFailures = @(Get-ChildItem -LiteralPath (Join-Path $evidence 'audit') -File -ErrorAction SilentlyContinue | Select-String 'controlled-child-binding-failed')
        if ($auditFailures.Count) { throw 'Audit contains controlled-child-binding-failed' }
        Write-Host "PASS $mode : descendant launch, inherited Profile, broker cleanup"
    } catch {
        $failures += "$mode : $($_.Exception.Message)"
        Write-Warning $failures[-1]
    } finally {
        # Every process here is a fixture PID created above; never stop existing Aura instances.
        if (!$child -and (Test-Path -LiteralPath (Join-Path $directory 'owned-child.pid'))) {
            $child = Get-Process -Id ([int](Get-Content -LiteralPath (Join-Path $directory 'owned-child.pid'))) -ErrorAction SilentlyContinue
        }
        if (!$root -and $mode -ne 'Normal' -and (Test-Path -LiteralPath (Join-Path $directory 'owned-root.pid'))) {
            $root = Get-Process -Id ([int](Get-Content -LiteralPath (Join-Path $directory 'owned-root.pid'))) -ErrorAction SilentlyContinue
        }
        if ($child) { if (!$child.HasExited) { $child.Kill(); $child.WaitForExit() }; $child.Dispose() }
        if ($root) { if (!$root.HasExited) { $root.Kill(); $root.WaitForExit() }; $root.Dispose() }
        if ($hostProcess) {
            if (!$hostProcess.HasExited) { $hostProcess.Kill(); $hostProcess.WaitForExit() }
            if ($stdout) { $stdout.Result | Set-Content -LiteralPath (Join-Path $directory 'cli.stdout.log') }
            if ($stderr) { $stderr.Result | Set-Content -LiteralPath (Join-Path $directory 'cli.stderr.log') }
            $hostProcess.Dispose()
        }
    }
}
if ($failures.Count) { throw "Detached session regression failed. $($failures -join '; '). Evidence: $evidence" }
