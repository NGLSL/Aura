param(
    [string]$RestoreRoot,
    [string]$MsvcRoot = 'D:/Tools/VS2022BuildTools/VC/Tools/MSVC/14.44.35207'
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (-not $RestoreRoot) { $RestoreRoot = Join-Path $repo 'target/driver-build-independent/restore' }
$wdk = Join-Path $RestoreRoot 'microsoft.windows.wdk.x64/10.0.26100.6584/c'
$sdk = Join-Path $RestoreRoot 'microsoft.windows.sdk.cpp/10.0.26100.6584/c'
$cl = Join-Path $MsvcRoot 'bin/Hostx64/x64/cl.exe'
$dumpbin = Join-Path $MsvcRoot 'bin/Hostx64/x64/dumpbin.exe'
foreach ($required in @($cl, $dumpbin, "$wdk/Include/10.0.26100.0/km/ntddk.h", "$sdk/Include/10.0.26100.0/shared", "$sdk/Include/10.0.26100.0/ucrt")) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Required cached build material missing: $required" }
}
$out = Join-Path $repo 'target/envbox-policy-kernel'
New-Item -ItemType Directory -Path $out -Force | Out-Null
$obj = Join-Path $out 'policy.obj'
$arguments = @('/nologo', '/c', '/kernel', '/W4', '/WX', '/GS-', '/Oi', '/Os', '/Zl', '/X', '/D_AMD64_=1', '/D_AMD64', "/I$wdk/Include/10.0.26100.0/km", "/I$wdk/Include/10.0.26100.0/shared", "/I$sdk/Include/10.0.26100.0/shared", "/I$sdk/Include/10.0.26100.0/ucrt", "/I$MsvcRoot/include", '/FIntddk.h', "/Fo$obj", (Join-Path $repo 'drivers/envbox-policy/policy.c'))
& $cl @arguments *> (Join-Path $out 'compile.log')
if ($LASTEXITCODE -ne 0) { throw 'Kernel object compile failed; see target/envbox-policy-kernel/compile.log' }
& $dumpbin /nologo /symbols $obj *> (Join-Path $out 'symbols.log')
if ($LASTEXITCODE -ne 0) { throw 'Kernel object symbol inspection failed' }
$undefined = @(Select-String -LiteralPath (Join-Path $out 'symbols.log') -Pattern '\bUNDEF\b')
if ($undefined.Count -ne 0) { throw 'Kernel object unexpectedly references external symbols; inspect symbols.log' }
[ordered]@{ scope = 'x64 kernel compilation only; no linked SYS or driver loaded'; compiler = $cl; wdk_headers = "$wdk/Include/10.0.26100.0/km"; compile_exit_code = 0; undefined_symbols = $undefined.Count; path = $obj; sha256 = (Get-FileHash -LiteralPath $obj -Algorithm SHA256).Hash } | ConvertTo-Json | Tee-Object -FilePath (Join-Path $out 'result.json')
