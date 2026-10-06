# Use the existing desktop Explorer as the launch broker so an elevated installer
# does not pass its administrator token to Aura. There is deliberately no elevated fallback.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$InstallDirectory)
$ErrorActionPreference = 'Stop'
try {
    if (-not [IO.Path]::IsPathRooted($InstallDirectory)) { throw 'Installation directory must be absolute.' }
    $directory = [IO.Path]::GetFullPath($InstallDirectory)
    $application = Join-Path $directory 'envbox-app.exe'
    if (-not (Test-Path -LiteralPath $application -PathType Leaf)) { throw 'Aura executable is missing.' }
    $shell = New-Object -ComObject 'Shell.Application'
    $desktop = $shell.Windows().FindWindowSW(0, 0, 8, [ref]0, 1)
    if ($null -eq $desktop) { throw 'Interactive Explorer desktop is unavailable.' }
    $desktop.Document.Application.ShellExecute($application, '', $directory, 'open', 1)
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
