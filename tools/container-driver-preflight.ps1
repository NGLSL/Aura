# Read-only qualification inventory. Never changes boot settings, installs drivers,
# creates certificates, enables Windows features, or assumes a VM is approved.
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
function Read-Check {
    param([scriptblock]$Action)
    try { @{ status = 'observed'; value = (& $Action) } }
    catch {
        @{ status = 'unverified'; error = $_.Exception.Message;
           error_id = $_.FullyQualifiedErrorId; hresult = $_.Exception.HResult }
    }
}

$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$report = [ordered]@{
    schema_version = 1
    observed_utc = [DateTime]::UtcNow.ToString('o')
    read_only = $true
    designated_test_environment = 'not provided; discovered VM inventory is not authorization'
    os = Read-Check {
        Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture
    }
    process_architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    elevated = Read-Check {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    }
    visual_studio = Read-Check {
        if (!(Test-Path -LiteralPath $vswhere)) { throw "vswhere missing: $vswhere" }
        $raw = & $vswhere -all -products '*' -format json
        if ($LASTEXITCODE -ne 0) { throw "vswhere exit: $LASTEXITCODE" }
        $raw | ConvertFrom-Json | ForEach-Object {
            $installation = $_
            @{ installation_path = $installation.installationPath;
               installation_version = $installation.installationVersion;
               product_id = $installation.productId;
               msvc = @(Get-ChildItem -LiteralPath (Join-Path $installation.installationPath 'VC\Tools\MSVC') -Directory |
                   ForEach-Object {
                       $compiler = Join-Path $_.FullName 'bin\Hostx64\x64\cl.exe'
                       @{ toolset = $_.Name; compiler = $compiler;
                          file_version = if (Test-Path -LiteralPath $compiler) { (Get-Item -LiteralPath $compiler).VersionInfo.FileVersion } else { $null } }
                   }) }
        }
    }
    kits = Read-Check {
        Get-ChildItem -LiteralPath (Join-Path $kits 'Include') -Directory | ForEach-Object {
            $version = $_.Name
            [ordered]@{
                version = $version
                kernel_header = Test-Path -LiteralPath (Join-Path $_.FullName 'km\ntddk.h')
                filter_header = Test-Path -LiteralPath (Join-Path $_.FullName 'km\fltKernel.h')
                kernel_library = Test-Path -LiteralPath (Join-Path $kits "Lib\$version\km\x64\ntoskrnl.lib")
                kernel_frameworks = Test-Path -LiteralPath (Join-Path $_.FullName 'wdf')
                signtool = Test-Path -LiteralPath (Join-Path $kits "bin\$version\x64\signtool.exe")
                signtool_file_version = if (Test-Path -LiteralPath (Join-Path $kits "bin\$version\x64\signtool.exe")) {
                    (Get-Item -LiteralPath (Join-Path $kits "bin\$version\x64\signtool.exe")).VersionInfo.FileVersion
                } else { $null }
                inf2cat = Test-Path -LiteralPath (Join-Path $kits "bin\$version\x64\Inf2Cat.exe")
            }
        }
    }
    secure_boot = Read-Check { Confirm-SecureBootUEFI }
    device_guard = Read-Check {
        Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard |
            Select-Object VirtualizationBasedSecurityStatus, SecurityServicesConfigured, SecurityServicesRunning
    }
    boot_configuration = Read-Check {
        $raw = & bcdedit.exe /enum 2>&1 | Out-String
        $code = $LASTEXITCODE
        @{ exit_code = $code; raw = $raw; readable = ($code -eq 0) }
    }
    virtualization_feature = Read-Check {
        Get-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V-All | Select-Object FeatureName, State
    }
    vm_inventory = Read-Check {
        if (!(Get-Command Get-VM -ErrorAction SilentlyContinue)) { throw 'Hyper-V Get-VM unavailable' }
        Get-VM | Select-Object Name, Id, State, Generation, CheckpointType
    }
    virtualization_commands = Read-Check {
        @('Get-VM', 'VBoxManage', 'vmrun', 'windbg', 'kd') | ForEach-Object {
            $name = $_
            @{ name = $name; found = [bool](Get-Command $name -ErrorAction SilentlyContinue) }
        }
    }
    code_signing_certificate_counts = Read-Check {
        # Counts only: do not export keys or record identity/thumbprint/private material.
        @('Cert:\CurrentUser\My', 'Cert:\LocalMachine\My') | ForEach-Object {
            $store = $_
            $valid = @(Get-ChildItem -LiteralPath $store -CodeSigningCert |
                Where-Object { $_.NotAfter -gt [DateTime]::Now -and $_.NotBefore -le [DateTime]::Now })
            @{ store = $store; currently_valid_code_signing_count = $valid.Count;
               ev_status = 'not inferred'; submission_access = 'not inferred' }
        }
    }
}
$report | ConvertTo-Json -Depth 12
