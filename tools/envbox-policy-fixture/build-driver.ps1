param(
    [string]$RestoreRoot,
    [string]$OutputDirectory,
    [string]$MsvcRoot = 'D:/Tools/VS2022BuildTools/VC/Tools/MSVC/14.44.35207'
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (-not $RestoreRoot) { $RestoreRoot = Join-Path $repo 'target/driver-build-independent/restore' }
$wdk = Join-Path $RestoreRoot 'microsoft.windows.wdk.x64/10.0.26100.6584/c'
$sdk = Join-Path $RestoreRoot 'microsoft.windows.sdk.cpp/10.0.26100.6584/c'
$bin = Join-Path $MsvcRoot 'bin/Hostx64/x64'
$out = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $repo ('target/envbox-policy-wfp-' + [Guid]::NewGuid().ToString('N')) }
New-Item -ItemType Directory -Path $out -Force | Out-Null

# Read-only SID computation. This neither creates nor starts the service.
$sidString = 'S-1-5-80-3820527054-1736232212-1554131609-2082016452-471345742'
$sidOutput = (& sc.exe showsid AuraPolicyService | Out-String)
if ($LASTEXITCODE -ne 0 -or $sidOutput -notmatch [regex]::Escape($sidString)) { throw 'Unexpected AuraPolicyService SID' }
$header = Get-Content -LiteralPath (Join-Path $repo 'drivers/envbox-policy/service_identity.h') -Raw
if (-not $header.Contains('L"' + $sidString + '"') -or -not $header.Contains('L"D:P(A;;GA;;;' + $sidString + ')"')) { throw 'Service SID/SDDL header mismatch' }
$sid = [System.Security.Principal.SecurityIdentifier]::new($sidString)
$bytes = New-Object byte[] $sid.BinaryLength
$sid.GetBinaryForm($bytes, 0)
$headerBytes = @([regex]::Matches($header, '0x[0-9A-Fa-f]{2}') | ForEach-Object { [Convert]::ToByte($_.Value.Substring(2),16) })
if ($bytes.Length -ne 32 -or $headerBytes.Length -ne 32 -or [Convert]::ToBase64String($bytes) -ne [Convert]::ToBase64String([byte[]]$headerBytes)) { throw 'Binary service SID header mismatch' }
$sidOutput | Set-Content -LiteralPath (Join-Path $out 'service-sid.log') -Encoding utf8

function Invoke-PolicyTool([string]$Name, [string[]]$Arguments, [string]$Log) {
    & (Join-Path $bin $Name) @Arguments *> (Join-Path $out $Log)
    if ($LASTEXITCODE -ne 0) { throw "$Name failed; see $out/$Log" }
}
$includes = @("/I$wdk/Include/10.0.26100.0/km", "/I$wdk/Include/10.0.26100.0/shared", "/I$sdk/Include/10.0.26100.0/shared", "/I$sdk/Include/10.0.26100.0/ucrt", "/I$MsvcRoot/include")
$flags = @('/nologo','/c','/kernel','/W4','/WX','/GS-','/Oi','/Os','/Zl','/X','/D_AMD64_=1','/D_AMD64')
foreach ($source in @('policy','network_snapshot','adapter','wfp')) {
    Invoke-PolicyTool 'cl.exe' ($flags + $includes + @("/Fo$out/$source.obj", (Join-Path $repo "drivers/envbox-policy/$source.c"))) "compile-$source.log"
}
$sys = Join-Path $out 'envbox-policy-prototype.sys'
$kmLib = Join-Path $wdk 'Lib/10.0.26100.0/km/x64'
Invoke-PolicyTool 'link.exe' @('/nologo','/driver','/subsystem:native','/entry:DriverEntry','/nodefaultlib','/machine:x64','/osversion:10.0','/integritycheck',"/out:$sys","$out/policy.obj","$out/network_snapshot.obj","$out/adapter.obj","$out/wfp.obj","$kmLib/ntoskrnl.lib","$kmLib/fwpkclnt.lib","$kmLib/wdmsec.lib","$kmLib/BufferOverflowK.lib") 'link.log'
Invoke-PolicyTool 'dumpbin.exe' @('/nologo','/headers',$sys) 'headers.log'
Invoke-PolicyTool 'dumpbin.exe' @('/nologo','/imports',$sys) 'imports.log'
$imports = Get-Content -LiteralPath (Join-Path $out 'imports.log') -Raw
$dlls = @([regex]::Matches($imports,'(?im)^\s+([\w.-]+\.(?:dll|exe|sys))\s*$') | ForEach-Object { $_.Groups[1].Value.ToLowerInvariant() } | Sort-Object -Unique)
if ($dlls.Count -eq 0 -or @($dlls | Where-Object { $_ -notin @('ntoskrnl.exe','hal.dll','fwpkclnt.sys') }).Count -or 'fwpkclnt.sys' -notin $dlls) { throw 'Unexpected kernel imports' }
$pe = [IO.File]::ReadAllBytes($sys)
$peOffset = [BitConverter]::ToInt32($pe,0x3c)
$machine = [BitConverter]::ToUInt16($pe,$peOffset+4)
$optional = $peOffset+24
$magic = [BitConverter]::ToUInt16($pe,$optional)
$subsystem = [BitConverter]::ToUInt16($pe,$optional+68)
$dllCharacteristics = [BitConverter]::ToUInt16($pe,$optional+70)
if ($machine -ne 0x8664 -or $magic -ne 0x20b -or $subsystem -ne 1 -or ($dllCharacteristics -band 0x80) -eq 0) { throw 'Unexpected PE machine/subsystem/integrity flags' }
[ordered]@{ completed = (Get-Date).ToUniversalTime().ToString('o'); scope = 'source-only WDM prototype; no service/driver installed or loaded; not unloadable'; sid = $sidString; compile_exit_code = 0; link_exit_code = 0; pe_machine = $machine; native_subsystem = $subsystem; force_integrity = $true; imports = $dlls; path = $sys; sha256 = (Get-FileHash -LiteralPath $sys -Algorithm SHA256).Hash } | ConvertTo-Json -Depth 5 | Tee-Object -FilePath (Join-Path $out 'result.json')
