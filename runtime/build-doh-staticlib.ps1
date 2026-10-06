param(
    [Parameter(Mandatory=$true)][string]$BuildDirectory,
    [Parameter(Mandatory=$true)][ValidateSet('x86_64-pc-windows-msvc','i686-pc-windows-msvc')][string]$RustTarget,
    [ValidateSet('Debug','Release','RelWithDebInfo','MinSizeRel')][string]$Configuration = 'Debug'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location -LiteralPath $repo
# Win32 MSBuild supplies an x86 LIB/INCLUDE environment, but Cargo also builds
# host (x64) build scripts. Let rustc/cc discover the appropriate toolchain for
# each target rather than inherit the C++ project's architecture globally.
foreach ($name in @('LIB','LIBPATH','INCLUDE','VSINSTALLDIR','VCINSTALLDIR','VCToolsInstallDir',
    'VSCMD_VER','VSCMD_ARG_TGT_ARCH','VSCMD_ARG_HOST_ARCH')) {
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
$env:PATH = (($env:PATH -split ';') | Where-Object {
    $_ -notmatch '(?i)[\\/]VC[\\/]Tools[\\/]MSVC[\\/].*[\\/]bin(?:[\\/]|$)'
}) -join ';'
New-Item -ItemType Directory -Path $BuildDirectory -Force | Out-Null
$product = Join-Path $BuildDirectory 'doh-product'
$stdout = Join-Path $BuildDirectory "doh-$Configuration.stdout.log"
$stderr = Join-Path $BuildDirectory "doh-$Configuration.stderr.log"
$arguments = @('+1.99.0', 'rustc', '-p', 'envbox-dns-doh', '--lib', '--target', $RustTarget,
    '--target-dir', $product, '--no-default-features', '--locked')
if ($Configuration -ne 'Debug') { $arguments += '--release' }
$arguments += @('--', '--print', 'native-static-libs')
# Start-Process preserves rustc's long native-static-libs line on PowerShell 5.1.
# Quotes are required for user workspaces containing spaces.
$quoted = $arguments | ForEach-Object { '"' + $_ + '"' }
$process = Start-Process -FilePath 'cargo.exe' -ArgumentList $quoted -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
if ($process.ExitCode -ne 0) { throw "DoH staticlib build failed; see $stderr" }
$line = Get-Content -LiteralPath $stderr | Where-Object { $_ -match 'native-static-libs:' } | Select-Object -Last 1
if (!$line) { throw 'Rust native-static-libs output is missing' }
$native = @(($line -replace '^.*native-static-libs:\s*','') -split '\s+' | Where-Object { $_ })
$metadataStdout = Join-Path $BuildDirectory 'doh-metadata.json'
$metadataStderr = Join-Path $BuildDirectory 'doh-metadata.stderr.log'
$metadataProcess = Start-Process -FilePath 'cargo.exe' -ArgumentList @('+1.99.0','metadata','--format-version','1','--locked','--offline') `
    -NoNewWindow -Wait -PassThru -RedirectStandardOutput $metadataStdout -RedirectStandardError $metadataStderr
if ($metadataProcess.ExitCode -ne 0) { throw "Cargo metadata failed; see $metadataStderr" }
$metadata = Get-Content -Raw -LiteralPath $metadataStdout | ConvertFrom-Json
$packageName = if ($RustTarget -eq 'x86_64-pc-windows-msvc') { 'windows_x86_64_msvc' } else { 'windows_i686_msvc' }
$package = $metadata.packages | Where-Object { $_.name -eq $packageName -and $_.version -eq '0.52.6' } | Select-Object -First 1
if (!$package) { throw 'Pinned windows-targets import archive package is missing' }
$archive = Join-Path (Split-Path $package.manifest_path) 'lib/windows.0.52.0.lib'
if (!(Test-Path -LiteralPath $archive)) { throw "Windows import archive is missing: $archive" }
$libraries = @($native | Where-Object { !$_.StartsWith('/') } | ForEach-Object {
    if ($_ -eq 'windows.0.52.0.lib') { $archive.Replace('\','/') } else { $_ }
})
$options = @($native | Where-Object { $_.StartsWith('/') })
$fragment = @('set(ENVBOX_DOH_NATIVE_LIBS') + @($libraries | ForEach-Object { '  "' + $_ + '"' }) + @(')') +
    @('set(ENVBOX_DOH_NATIVE_OPTIONS') + @($options | ForEach-Object { '  "' + $_ + '"' }) + @(')')
$fragmentPath = Join-Path $BuildDirectory 'doh-native-libs.cmake'
$content = ($fragment -join [Environment]::NewLine) + [Environment]::NewLine
# This file is a CMake configure dependency. Rewriting identical content from
# the build target would trigger another configure during every build.
if (!(Test-Path -LiteralPath $fragmentPath) -or
    (Get-Content -Raw -LiteralPath $fragmentPath) -ne $content) {
    Set-Content -LiteralPath $fragmentPath -Value $content -Encoding utf8 -NoNewline
}
