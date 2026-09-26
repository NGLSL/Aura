# Legacy entry point — Kite-style packaging now lives in build-installer.ps1 (NSIS).
# Usage:  .\scripts\make-installer.ps1 [-SkipBuild] [-Version 0.3.0]
param(
    [string]$Version = "0.3.0",
    [switch]$SkipBuild
)
& (Join-Path $PSScriptRoot "build-installer.ps1") -Version $Version -SkipBuild:$SkipBuild
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
