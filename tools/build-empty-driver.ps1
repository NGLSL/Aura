# Build and inspect the intentionally inert WDM driver fixture without
# installing WDK system-wide, changing boot policy, or loading a driver.
[CmdletBinding()]
param(
    [ValidateSet('x64')]
    [string]$Architecture = 'x64',
    [switch]$NoRestore
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$buildRoot = Join-Path $repo 'target\driver-build-independent'
$restoreRoot = Join-Path $buildRoot 'restore'
$outputRoot = Join-Path $buildRoot 'output\x64'
$objectRoot = Join-Path $buildRoot 'obj\x64'
New-Item -ItemType Directory -Path $restoreRoot, $outputRoot, $objectRoot -Force | Out-Null

$packageSpecs = @(
    [ordered]@{
        id = 'microsoft.windows.wdk.x64'
        version = '10.0.26100.6584'
        sha512 = '8E175D6819E1303AADDC656BDF64554ED691D0A1E66438D8D09093327D74390A64E3E01285708AF50034F6F05ACE9F278B5323BBCE2A3394865DF21F9F2389FA'
        size = 110872506
    },
    [ordered]@{
        id = 'microsoft.windows.sdk.cpp.x64'
        version = '10.0.26100.6584'
        sha512 = 'FB913010BC0EBEC4B3806AC70D0D2CB5D68EB5864719F27D72FC7D6CDE83F3C2B3394F892EC14BD5B10A1382BB53491DF7DAEF6BFBAFD4FE5A0EF41644283B39'
        size = 52245405
    },
    [ordered]@{
        id = 'microsoft.windows.sdk.cpp'
        version = '10.0.26100.6584'
        sha512 = '2AB1D73514F4B2BDC1AA6BD5062AF467F72BD48EBA32F999E984F005ACE2D13CFB3F7E7AB91082CA78E5DFFAD9C2C4B424B602743FFE7E54D2375D35ADC9A6FA'
        size = 160036542
    }
)

function Invoke-RequiredTool {
    param(
        [Parameter(Mandatory)] [string]$FilePath,
        [Parameter(Mandatory)] [string[]]$ArgumentList,
        [Parameter(Mandatory)] [string]$LogPath
    )

    $text = (& $FilePath @ArgumentList 2>&1 | Out-String)
    $code = $LASTEXITCODE
    Set-Content -LiteralPath $LogPath -Value $text -Encoding utf8
    if ($code -ne 0) {
        throw "$(Split-Path -Leaf $FilePath) failed with exit code $code. See $LogPath"
    }
    return [ordered]@{ exit_code = $code; log = $LogPath }
}

function Find-File {
    param(
        [Parameter(Mandatory)] [string[]]$Candidates,
        [Parameter(Mandatory)] [string]$Description
    )

    foreach ($candidate in $Candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw "Unable to locate $Description. Tried: $($Candidates -join '; ')"
}

function Restore-Package {
    param([Parameter(Mandatory)] [hashtable]$Spec)

    $idRoot = Join-Path $restoreRoot $Spec.id
    $packagePath = Join-Path $idRoot "$($Spec.version).nupkg"
    $extractRoot = Join-Path $idRoot $Spec.version
    New-Item -ItemType Directory -Path $idRoot -Force | Out-Null

    if (!(Test-Path -LiteralPath $packagePath -PathType Leaf)) {
        if ($NoRestore) {
            throw "NuGet package is absent and -NoRestore was supplied: $packagePath"
        }
        $uri = "https://api.nuget.org/v3-flatcontainer/$($Spec.id)/$($Spec.version)/$($Spec.id).$($Spec.version).nupkg"
        Write-Host "Downloading $uri"
        Invoke-WebRequest -Uri $uri -OutFile $packagePath -UseBasicParsing
    }

    $file = Get-Item -LiteralPath $packagePath
    $actualHash = (Get-FileHash -LiteralPath $packagePath -Algorithm SHA512).Hash.ToUpperInvariant()
    if ($file.Length -ne $Spec.size -or $actualHash -ne $Spec.sha512) {
        throw "NuGet package integrity mismatch for $($Spec.id) $($Spec.version): size=$($file.Length), sha512=$actualHash"
    }

    $marker = Join-Path $extractRoot '.envbox-restore.json'
    if (!(Test-Path -LiteralPath $marker -PathType Leaf)) {
        if (Test-Path -LiteralPath $extractRoot) {
            throw "Existing extraction has no integrity marker; remove only this version directory before retrying: $extractRoot"
        }
        New-Item -ItemType Directory -Path $extractRoot -Force | Out-Null
        Expand-Archive -LiteralPath $packagePath -DestinationPath $extractRoot -Force
        [ordered]@{
            package = $Spec.id
            version = $Spec.version
            sha512 = $actualHash
            size = $file.Length
        } | ConvertTo-Json | Set-Content -LiteralPath $marker -Encoding utf8
    } else {
        $saved = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
        if ($saved.sha512 -ne $actualHash -or $saved.version -ne $Spec.version) {
            throw "NuGet extraction marker mismatch: $marker"
        }
    }

    return [ordered]@{
        id = $Spec.id
        version = $Spec.version
        path = $packagePath
        extracted = $extractRoot
        size = $file.Length
        sha512 = $actualHash
    }
}

$restored = @($packageSpecs | ForEach-Object { Restore-Package $_ })
$wdk = $restored | Where-Object id -eq 'microsoft.windows.wdk.x64' | Select-Object -First 1
$sdkX64 = $restored | Where-Object id -eq 'microsoft.windows.sdk.cpp.x64' | Select-Object -First 1
$sdk = $restored | Where-Object id -eq 'microsoft.windows.sdk.cpp' | Select-Object -First 1

$wdkC = Join-Path $wdk.extracted 'c'
$sdkC = Join-Path $sdk.extracted 'c'
$sdkX64C = Join-Path $sdkX64.extracted 'c'
$kitVersion = '10.0.26100.0'
$wdkInclude = Join-Path $wdkC "Include\$kitVersion\km"
$wdkShared = Join-Path $wdkC "Include\$kitVersion\shared"
$sdkShared = Join-Path $sdkC "Include\$kitVersion\shared"
$sdkUm = Join-Path $sdkC "Include\$kitVersion\um"
$sdkUcrt = Join-Path $sdkC "Include\$kitVersion\ucrt"
$ntoskrnl = Join-Path $wdkC "Lib\$kitVersion\km\x64\ntoskrnl.lib"
$inf2cat = Join-Path $wdkC "bin\$kitVersion\x86\Inf2Cat.exe"
$infverif = Join-Path $wdkC "tools\$kitVersion\x64\infverif.exe"
$signtool = Join-Path $sdkC "bin\$kitVersion\x64\signtool.exe"
foreach ($required in @($wdkInclude, $sdkShared, $sdkUm, $sdkUcrt, $ntoskrnl, $inf2cat, $infverif, $signtool)) {
    if (!(Test-Path -LiteralPath $required)) { throw "Restored package is missing required path: $required" }
}

$vsCandidates = @()
if ($env:VSINSTALLDIR) { $vsCandidates += $env:VSINSTALLDIR }
$vsCandidates += @('D:\Tools\VS2022BuildTools')
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $vsCandidates += @(& $vswhere -latest -products '*' -property installationPath 2>$null)
}
$vsRoot = $vsCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (!$vsRoot) { throw 'Visual Studio Build Tools installation was not found' }
$msvcRoot = Join-Path $vsRoot 'VC\Tools\MSVC'
$msvcVersion = Get-ChildItem -LiteralPath $msvcRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1
if (!$msvcVersion) { throw "MSVC toolset was not found under $msvcRoot" }
$msvc = $msvcVersion.FullName
$cl = Find-File @((Join-Path $msvc 'bin\Hostx64\x64\cl.exe')) 'x64 MSVC compiler'
$link = Find-File @((Join-Path $msvc 'bin\Hostx64\x64\link.exe')) 'x64 MSVC linker'
$dumpbin = Find-File @((Join-Path $msvc 'bin\Hostx64\x64\dumpbin.exe')) 'x64 dumpbin'
$msvcInclude = Join-Path $msvc 'include'
if (!(Test-Path -LiteralPath $msvcInclude)) { throw "MSVC include directory was not found: $msvcInclude" }

$source = Join-Path $repo 'drivers\envbox-empty\envbox_empty.c'
$inf = Join-Path $repo 'drivers\envbox-empty\envbox-empty.inf'
foreach ($input in @($source, $inf)) {
    if (!(Test-Path -LiteralPath $input -PathType Leaf)) { throw "Fixture input is missing: $input" }
}
$object = Join-Path $objectRoot 'envbox_empty.obj'
$sys = Join-Path $outputRoot 'envbox-empty.sys'
$pdb = Join-Path $outputRoot 'envbox-empty.pdb'
$outputInf = Join-Path $outputRoot 'envbox-empty.inf'
$compileLog = Join-Path $buildRoot 'compile.log'
$linkLog = Join-Path $buildRoot 'link.log'
$infLog = Join-Path $buildRoot 'inf2cat.log'
$infVerifLog = Join-Path $buildRoot 'infverif.log'
$infVerifWhqlLog = Join-Path $buildRoot 'infverif-whql.log'
$dumpHeadersLog = Join-Path $buildRoot 'dumpbin-headers.log'
$dumpImportsLog = Join-Path $buildRoot 'dumpbin-imports.log'
$signLog = Join-Path $buildRoot 'signtool-verify.log'
Copy-Item -LiteralPath $inf -Destination $outputInf -Force

$oldPath = $env:Path
try {
    $env:Path = "$(Split-Path -Parent $cl);$oldPath"
    $compile = Invoke-RequiredTool -FilePath $cl -ArgumentList @(
        '/nologo', '/c', '/kernel', '/W4', '/WX', '/GS-', '/Oi', '/Os', '/Zl', '/X',
        '/D_AMD64_=1', '/D_AMD64',
        "/I$wdkInclude", "/I$wdkShared", "/I$sdkShared", "/I$sdkUm", "/I$sdkUcrt", "/I$msvcInclude",
        "/Fo$object", $source
    ) -LogPath $compileLog

    $linkResult = Invoke-RequiredTool -FilePath $link -ArgumentList @(
        '/nologo', '/driver', '/subsystem:native', '/entry:DriverEntry', '/nodefaultlib',
        '/machine:x64', '/osversion:10.0', "/out:$sys", "/pdb:$pdb", $object, $ntoskrnl
    ) -LogPath $linkLog

    $infResult = Invoke-RequiredTool -FilePath $inf2cat -ArgumentList @(
        "/driver:$outputRoot", '/os:10_X64', '/verbose'
    ) -LogPath $infLog
    $infVerifResult = Invoke-RequiredTool -FilePath $infverif -ArgumentList @(
        '/w', $outputInf
    ) -LogPath $infVerifLog
    $infVerifWhqlResult = Invoke-RequiredTool -FilePath $infverif -ArgumentList @(
        '/h', $outputInf
    ) -LogPath $infVerifWhqlLog

    $headers = (& $dumpbin /nologo /headers $sys 2>&1 | Out-String)
    $headersCode = $LASTEXITCODE
    Set-Content -LiteralPath $dumpHeadersLog -Value $headers -Encoding utf8
    if ($headersCode -ne 0) { throw "dumpbin /headers failed with exit code $headersCode" }
    $imports = (& $dumpbin /nologo /imports $sys 2>&1 | Out-String)
    $importsCode = $LASTEXITCODE
    Set-Content -LiteralPath $dumpImportsLog -Value $imports -Encoding utf8
    if ($importsCode -ne 0) { throw "dumpbin /imports failed with exit code $importsCode" }

    $signText = (& $signtool verify /kp $sys 2>&1 | Out-String)
    $signCode = $LASTEXITCODE
    Set-Content -LiteralPath $signLog -Value $signText -Encoding utf8
    $authenticode = Get-AuthenticodeSignature -LiteralPath $sys

    $sysHash = (Get-FileHash -LiteralPath $sys -Algorithm SHA256).Hash
    $cat = Join-Path $outputRoot 'envbox-empty.cat'
    $catHash = (Get-FileHash -LiteralPath $cat -Algorithm SHA256).Hash
    $report = [ordered]@{
        schema_version = 1
        observed_utc = [DateTime]::UtcNow.ToString('o')
        read_only_host = $true
        architecture = 'x64'
        package_restore = $restored
        package_paths = [ordered]@{ wdk = $wdkC; sdk_x64 = $sdkX64C; sdk = $sdkC }
        kit_version = $kitVersion
        visual_studio = [ordered]@{
            installation = $vsRoot
            msvc_toolset = $msvcVersion.Name
            compiler = $cl
            linker = $link
            dumpbin = $dumpbin
        }
        tools = [ordered]@{
            inf2cat = $inf2cat
            infverif = $infverif
            signtool = $signtool
            compile = $compile
            link = $linkResult
            inf2cat_run = $infResult
            infverif_run = $infVerifResult
            infverif_whql_run = $infVerifWhqlResult
        }
        outputs = [ordered]@{
            sys = $sys
            sys_sha256 = $sysHash
            sys_bytes = (Get-Item -LiteralPath $sys).Length
            cat = $cat
            cat_sha256 = $catHash
            cat_bytes = (Get-Item -LiteralPath $cat).Length
        }
        pe_inspection = [ordered]@{
            headers_log = $dumpHeadersLog
            imports_log = $dumpImportsLog
            machine_x64 = $headers -match '8664 machine \(x64\)'
            native_subsystem = $headers -match 'subsystem \(Native\)'
            import_directory_empty = $headers -match '0 \[\s*0\] RVA \[\s*size\s*\].*Import Directory'
            certificate_table_empty = $headers -match '0 \[\s*0\] RVA \[\s*size\s*\].*Certificates Directory'
            imports_log_has_no_import_table = $imports -notmatch '(?i)DLL Name|Import Name'
        }
        signing = [ordered]@{
            status = if ($authenticode.Status.ToString() -eq 'NotSigned') { 'unsigned' } else { $authenticode.Status.ToString() }
            authenticode_status = $authenticode.Status.ToString()
            signtool_exit_code = $signCode
            signtool_log = $signLog
            loadable_claim = 'not_claimed; no driver was installed or loaded'
        }
        safety = [ordered]@{
            installed = $false
            loaded = $false
            boot_changed = $false
            test_signing_changed = $false
            verifier_changed = $false
            certificates_created_or_imported = $false
        }
    }
    $reportPath = Join-Path $buildRoot 'result.json'
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportPath -Encoding utf8
    $report | ConvertTo-Json -Depth 12
    # signtool is intentionally expected to return 1 for this unsigned fixture;
    # expose the build result itself as successful to the calling PowerShell.
    $global:LASTEXITCODE = 0
} finally {
    $env:Path = $oldPath
}
