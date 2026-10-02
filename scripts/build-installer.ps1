# Build the complete Aura / EnvBox NSIS installer.
# Run Cargo and CMake for all packaged binaries; their dependency tracking
# rebuilds only targets whose inputs changed.
[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$Nsis = ""
)
$ErrorActionPreference = "Stop"
$packagingTimer = [System.Diagnostics.Stopwatch]::StartNew()
$root = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $root "Cargo.toml"
$metadataJson = & cargo metadata --locked --no-deps --format-version 1 --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed ($LASTEXITCODE)" }
$metadata = ($metadataJson -join "`n") | ConvertFrom-Json
$appPackage = @($metadata.packages | Where-Object { $_.name -eq "envbox-app" })
if ($appPackage.Count -ne 1) { throw "expected exactly one envbox-app package in Cargo metadata" }
$cargoVersion = [string]$appPackage[0].version
if ($cargoVersion -notmatch '^\d+\.\d+\.\d+$') { throw "unsupported Cargo version: $cargoVersion" }
if ($Version -and $Version -ne $cargoVersion) {
    throw "installer version $Version differs from envbox-app version $cargoVersion; update Cargo.toml and Cargo.lock"
}
$Version = $cargoVersion

$artifacts = Join-Path $root "artifacts"
$rel = Join-Path $root "target\release"
New-Item -ItemType Directory -Force $artifacts | Out-Null

function Find-CMake {
    $candidates = @(
        "D:\Tools\VS2022BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    $command = Get-Command cmake.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    return $null
}

$cmake = Find-CMake
if (-not $cmake) { throw "cmake not found. Install VS Build Tools CMake or put cmake on PATH." }
$cargo = (Get-Command cargo -ErrorAction Stop).Source
$detours = $env:DETOURS_ROOT
if (-not $detours) { $detours = "D:\Tools\Detours" }
if (-not (Test-Path -LiteralPath $detours -PathType Container)) {
    throw "Detours not found at $detours. Set DETOURS_ROOT or build Detours first."
}

$setup = Join-Path $artifacts "aura-setup.exe"
if (Test-Path -LiteralPath $setup -PathType Leaf) {
    Remove-Item -LiteralPath $setup -Force
}

# Independent builds reuse their existing output directories. Stage only after
# all three succeed, including when another build fails while they are running.
$buildTask = {
    param($Name, $Root, $Cargo, $CMake, $Detours, $CMakeArch, $BuildDirName, $DllName)
    $ErrorActionPreference = "Stop"
    Set-Location -LiteralPath $Root
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    function Invoke-BuildCommand {
        param([string]$Command, [string[]]$Arguments, [string]$Stage)
        # Native stderr is build output, not a PowerShell terminating error.
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = $null
        & $Command @Arguments 2>&1 | ForEach-Object { "[$Name] $_" }
        if ($null -eq $LASTEXITCODE) { throw "$Stage could not start: $Command" }
        if ($LASTEXITCODE -ne 0) { throw "$Stage failed ($LASTEXITCODE)" }
    }
    try {
        if ($Name -eq "cargo") {
            Invoke-BuildCommand $Cargo @("build", "--workspace", "--release", "--locked", "--manifest-path", (Join-Path $Root "Cargo.toml")) "cargo release build"
        } else {
            $buildDir = Join-Path $Root "target\$BuildDirName"
            Invoke-BuildCommand $CMake @("-S", (Join-Path $Root "runtime"), "-B", $buildDir, "-G", "Visual Studio 17 2022", "-A", $CMakeArch, "-DDETOURS_ROOT=$Detours") "cmake configure for $Name"
            Invoke-BuildCommand $CMake @("--build", $buildDir, "--config", "Release", "--target", "envbox-$DllName") "cmake build for $Name"
            $dll = Join-Path $buildDir "Release\envbox-$DllName.dll"
            if (-not (Test-Path -LiteralPath $dll -PathType Leaf)) {
                throw "runtime build succeeded but output is missing: $dll"
            }
        }
    } finally {
        "[$Name] elapsed: $([math]::Round($timer.Elapsed.TotalSeconds, 1)) s"
    }
}

$buildTimer = [System.Diagnostics.Stopwatch]::StartNew()
$jobs = @()
try {
    Write-Host "== parallel release builds: cargo, runtime x64, runtime x86 =="
    $jobs += Start-Job -Name "cargo" -ScriptBlock $buildTask -ArgumentList "cargo", $root, $cargo, $cmake, $detours
    $jobs += Start-Job -Name "runtime-x64" -ScriptBlock $buildTask -ArgumentList "runtime-x64", $root, $cargo, $cmake, $detours, "x64", "runtime-build", "runtime64"
    $jobs += Start-Job -Name "runtime-x86" -ScriptBlock $buildTask -ArgumentList "runtime-x86", $root, $cargo, $cmake, $detours, "Win32", "runtime-build32", "runtime32"
    do {
        $running = @($jobs | Where-Object { $_.State -eq "Running" -or $_.State -eq "NotStarted" })
        if ($running.Count) { Wait-Job -Job $running -Any -Timeout 1 | Out-Null }
        $jobs | Receive-Job -ErrorAction SilentlyContinue | ForEach-Object { Write-Host $_ }
    } while ($running.Count)
    $failed = @($jobs | Where-Object { $_.State -ne "Completed" })
    if ($failed.Count) {
        $details = $failed | ForEach-Object { "$($_.Name): $($_.ChildJobs[0].JobStateInfo.Reason.Message)" }
        throw "release builds failed: $($details -join '; ')"
    }
} finally {
    # Let native tools finish before removing their job hosts, even on failure.
    if ($jobs.Count) {
        $jobs | Wait-Job | Out-Null
        $jobs | Receive-Job -ErrorAction SilentlyContinue | ForEach-Object { Write-Host $_ }
        $jobs | Remove-Job
    }
    Write-Host "Release builds elapsed: $([math]::Round($buildTimer.Elapsed.TotalSeconds, 1)) s"
}

New-Item -ItemType Directory -Force $rel | Out-Null
foreach ($runtime in @(
    @{ BuildDir = "runtime-build"; Dll = "envbox-runtime64.dll" },
    @{ BuildDir = "runtime-build32"; Dll = "envbox-runtime32.dll" }
)) {
    $dll = Join-Path $root "target\$($runtime.BuildDir)\Release\$($runtime.Dll)"
    Copy-Item -LiteralPath $dll -Destination (Join-Path $rel $runtime.Dll) -Force
}

# Stage every file expected by the NSIS script from this release build.
$bins = @(
    "envbox-app.exe",
    "envbox.exe",
    "envbox-broker.exe",
    "envbox-probe.exe",
    "envbox-browser-probe.exe",
    "envbox-suspended-helper.exe",
    "envbox-runtime64.dll",
    "envbox-runtime32.dll"
)
foreach ($name in $bins) {
    $src = Join-Path $rel $name
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) {
        throw "missing release binary: $src"
    }
    Copy-Item -LiteralPath $src -Destination (Join-Path $artifacts $name) -Force
    Write-Host "  staged $name"
}

if (-not $Nsis) {
    $Nsis = @(
        (Get-Command makensis.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
        "C:\Program Files (x86)\NSIS\makensis.exe",
        "C:\Program Files\NSIS\makensis.exe"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Nsis) { throw "NSIS not found. Install with: winget install NSIS.NSIS" }

$nsi = Join-Path $root "installer\aura.nsi"
Write-Host "== makensis $nsi =="
& $Nsis "/DVERSION=$Version" $nsi
if ($LASTEXITCODE -ne 0) { throw "makensis failed ($LASTEXITCODE)" }

if (-not (Test-Path -LiteralPath $setup)) { throw "expected setup missing: $setup" }
$size = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host ""
Write-Host "Installer: $setup (${size} MB)  version=$Version"
Write-Host "Total packaging elapsed: $([math]::Round($packagingTimer.Elapsed.TotalSeconds, 1)) s"
Write-Host "Silent:    `"$setup`" /S"
