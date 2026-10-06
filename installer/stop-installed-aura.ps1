# Invoked only by the installer/uninstaller. Never terminates an application tree.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$InstallDirectory)
$ErrorActionPreference = 'Stop'
try {
    if (-not [IO.Path]::IsPathRooted($InstallDirectory)) { throw 'Installation directory must be absolute.' }
    $directory = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\')
    if ($directory -eq [IO.Path]::GetPathRoot($directory).TrimEnd('\')) { throw 'A drive root is not an installation directory.' }
    if (Test-Path -LiteralPath $directory) {
        $item = Get-Item -LiteralPath $directory -Force
        if (-not $item.PSIsContainer) { throw 'Installation path is not a directory.' }
        # Refuse aliases rather than risk stopping processes from another installation.
        for ($ancestor = $item; $null -ne $ancestor; $ancestor = $ancestor.Parent) {
            if ($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Installation path contains a reparse point.' }
        }
    }
    $names = @('envbox-app.exe', 'envbox.exe', 'envbox-broker.exe', 'envbox-supervisor.exe',
        'envbox-probe.exe', 'envbox-browser-probe.exe', 'envbox-suspended-helper.exe', 'envbox-suspended-helper32.exe')
    $expected = @{}
    foreach ($name in $names) { $expected[(Join-Path $directory $name)] = $true }
    # Stabilize until no matching managers remain: stopping the GUI may race its last worker startup.
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $matching = @(Get-CimInstance -ClassName Win32_Process | Where-Object {
            $names -contains $_.Name -and $_.ExecutablePath -and $expected.ContainsKey([IO.Path]::GetFullPath($_.ExecutablePath))
        })
        foreach ($candidate in $matching) {
            # Bind to this process object and recheck path immediately before termination.
            $process = Get-Process -Id $candidate.ProcessId -ErrorAction SilentlyContinue
            if ($null -eq $process) { continue }
            try {
                if ($process.HasExited) { continue }
                if (-not $expected.ContainsKey([IO.Path]::GetFullPath($process.Path))) { throw "Process path changed for PID $($candidate.ProcessId)." }
                $process.Kill()
                if (-not $process.WaitForExit(5000)) { throw "Process $($candidate.Name) did not exit." }
            } finally { $process.Dispose() }
        }
        Start-Sleep -Milliseconds 150
        $remaining = @(Get-CimInstance -ClassName Win32_Process | Where-Object {
            $names -contains $_.Name -and $_.ExecutablePath -and $expected.ContainsKey([IO.Path]::GetFullPath($_.ExecutablePath))
        })
        if ($remaining.Count -eq 0) {
            # A target application may still map a Runtime DLL. Never terminate it;
            # block installation before writes and ask the user to close it instead.
            foreach ($name in ($names + @('envbox-runtime64.dll', 'envbox-runtime32.dll'))) {
                $file = Join-Path $directory $name
                if (Test-Path -LiteralPath $file -PathType Leaf) {
                    $stream = [IO.File]::Open($file, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
                    $stream.Dispose()
                }
            }
            exit 0
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Aura processes are still running in the installation directory.'
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
