param([switch]$ContractOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$nsi = Get-Content -LiteralPath (Join-Path $root 'installer/aura.nsi') -Raw
if ($nsi -match 'taskkill[^\r\n]+/IM') { throw 'Unsafe global process-name termination remains in installer.' }
if ($nsi -notmatch 'MUI_FINISHPAGE_RUN_FUNCTION') { throw 'Finish page does not offer Aura launch.' }
if ($nsi -notmatch 'Call un.StopInstalledAura') { throw 'Uninstall does not stop installed Aura processes before deleting files.' }
if ($nsi -notmatch 'Sysnative\\WindowsPowerShell') { throw 'The x86 installer does not select native PowerShell for x64 process inspection.' }
if ($nsi -match 'MUI_FINISHPAGE_RUN_NOTCHECKED') { throw 'Aura launch should be checked by default.' }
if ($nsi.IndexOf('Call StopInstalledAura') -gt $nsi.IndexOf('File "..\artifacts\envbox-app.exe"')) { throw 'Process shutdown runs after payload extraction.' }
if ($ContractOnly) { Write-Host 'Installer contracts passed.'; exit 0 }
$fixture = Join-Path $root ('target/installer-upgrade-' + [Guid]::NewGuid().ToString('N'))
$first = Join-Path $fixture 'first'
$other = Join-Path $fixture 'other'
New-Item -ItemType Directory -Path $first, $other -Force | Out-Null
$source = @'
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
class Fixture {
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr p, uint a, out IntPtr t);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool GetTokenInformation(IntPtr t, int c, out int e, int n, out int r);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    public static void Main(string[] args) {
        string directory = Path.GetDirectoryName(System.Reflection.Assembly.GetExecutingAssembly().Location);
        IntPtr token; int elevated, returned;
        if (!OpenProcessToken(Process.GetCurrentProcess().Handle, 8, out token)) throw new Exception("token");
        try { if (!GetTokenInformation(token, 20, out elevated, 4, out returned)) throw new Exception("elevation"); }
        finally { CloseHandle(token); }
        File.WriteAllText(Path.Combine(directory, "ready-" + Process.GetCurrentProcess().ProcessName + ".txt"),
            Process.GetCurrentProcess().Id + "\n" + elevated + "\n" + WindowsIdentity.GetCurrent().User.Value);
        if (args.Length > 1) { Process.Start(new ProcessStartInfo(args[1]) { UseShellExecute = false, CreateNoWindow = true }); }
        if (args.Length > 0) {
            using (var file = File.Open(args[0], FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None)) {
                File.WriteAllText(args[0] + ".ready", "locked");
                System.Threading.Thread.Sleep(60000);
            }
        } else { System.Threading.Thread.Sleep(60000); }
    }
}
'@
$exe = Join-Path $first 'envbox-supervisor.exe'
Add-Type -TypeDefinition $source -OutputAssembly $exe -OutputType WindowsApplication
Copy-Item -LiteralPath $exe -Destination (Join-Path $other 'envbox-supervisor.exe')
Copy-Item -LiteralPath $exe -Destination (Join-Path $first 'ordinary-target.exe')
Copy-Item -LiteralPath $exe -Destination (Join-Path $first 'envbox-app.exe')
$owned = @()
function Invoke-Helper([string]$Name, [string]$Directory) {
    $process = Start-Process -FilePath "$env:WINDIR/System32/WindowsPowerShell/v1.0/powershell.exe" -WindowStyle Hidden -Wait -PassThru -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"' + (Join-Path $root ('installer/' + $Name)) + '"'), '-InstallDirectory', ('"' + $Directory + '"')) -RedirectStandardOutput (Join-Path $fixture ($Name + '.stdout')) -RedirectStandardError (Join-Path $fixture ($Name + '.stderr'))
    return $process.ExitCode
}
try {
    $lock = Join-Path $first 'locked.bin'
    $targetExe = Join-Path $first 'ordinary-target.exe'
    $manager = Start-Process -FilePath $exe -ArgumentList ('"' + $lock + '" "' + $targetExe + '"') -WindowStyle Hidden -PassThru
    $otherManager = Start-Process -FilePath (Join-Path $other 'envbox-supervisor.exe') -WindowStyle Hidden -PassThru
    $owned += $manager, $otherManager
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $targetMarker = Join-Path $first 'ready-ordinary-target.txt'
    while (-not (Test-Path -LiteralPath ($lock + '.ready')) -or -not (Test-Path -LiteralPath $targetMarker)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Fixture did not acquire file lock.' }
        Start-Sleep -Milliseconds 100
    }
    $target = Get-Process -Id ([int](Get-Content -LiteralPath $targetMarker -First 1))
    $owned += $target
    $red = $false
    try { $stream = [IO.File]::Open($lock, 'Open', 'ReadWrite', 'None'); $stream.Dispose() } catch [IO.IOException] { $red = $true }
    if (-not $red) { throw 'RED: running supervisor failed to hold the fixture lock.' }
    Write-Host 'RED confirmed: running supervisor prevents overwrite.'
    if ((Invoke-Helper 'stop-installed-aura.ps1' 'relative-path') -eq 0) { throw 'Relative path was accepted.' }
    if ($manager.HasExited -or $otherManager.HasExited -or $target.HasExited) { throw 'Invalid path stopped a process.' }
    if ((Invoke-Helper 'stop-installed-aura.ps1' ($first + '\.')) -ne 0) { throw 'Installed-manager shutdown failed.' }
    $manager.Refresh(); $otherManager.Refresh(); $target.Refresh()
    if (-not $manager.HasExited) { throw 'Target-directory supervisor survived.' }
    if ($otherManager.HasExited -or $target.HasExited) { throw 'Another installation or target application was terminated.' }
    $stream = [IO.File]::Open($lock, 'Open', 'ReadWrite', 'None'); $stream.Dispose()
    Write-Host 'GREEN: target supervisor stopped, file released, same-name other installation and ordinary target preserved.'
    $runtime = Join-Path $first 'envbox-runtime64.dll'
    $held = [IO.File]::Open($runtime, 'OpenOrCreate', 'ReadWrite', 'None')
    try {
        if ((Invoke-Helper 'stop-installed-aura.ps1' $first) -eq 0) { throw 'Locked runtime incorrectly passed preflight.' }
        if ($target.HasExited) { throw 'Runtime preflight killed a target application.' }
    } finally { $held.Dispose() }
    if ((Invoke-Helper 'launch-installed-aura.ps1' $first) -ne 0) { throw 'Explorer launch failed.' }
    $marker = Join-Path $first 'ready-envbox-app.txt'
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $marker)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Explorer did not launch the app.' }
        Start-Sleep -Milliseconds 100
    }
    $lines = Get-Content -LiteralPath $marker
    $launched = Get-Process -Id ([int]$lines[0]); $owned += $launched
    if ($lines[1] -ne '0') { throw 'Explorer-launched app has an elevated token.' }
    $interactiveUserMatches = [string]$lines[2] -eq [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if (-not $interactiveUserMatches) { throw 'Explorer-launched app does not belong to the interactive test user.' }
    $evidence = [ordered]@{ Fixture=$fixture; TargetStopped=$manager.HasExited; OtherInstallationPreserved=(-not $otherManager.HasExited); TargetChildApplicationPreserved=(-not $target.HasExited); AppPid=[int]$lines[0]; AppElevated=[int]$lines[1]; InteractiveUserMatches=$interactiveUserMatches; Passed=$true }
    $evidence | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture 'result.json') -Encoding utf8
    Write-Host ('Installer upgrade fixtures passed. Evidence: ' + (Join-Path $fixture 'result.json'))
} finally {
    foreach ($process in $owned) {
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
}
