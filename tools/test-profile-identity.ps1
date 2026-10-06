param(
    [Parameter(Mandatory=$true)][string]$RuntimeDirectory,
    [string]$Cli = '',
    [string]$Probe64 = '',
    [string]$Probe32 = '',
    [string]$Prefix = 'profile-identity'
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($Prefix -notmatch '^[a-z0-9-]+$') { throw 'Invalid evidence prefix' }
$loaded = @([Diagnostics.Process]::GetCurrentProcess().Modules | Where-Object ModuleName -Like 'envbox-runtime*')
if ($loaded.Count -ne 0) { throw 'Acceptance controller must be a fresh, uninjected Windows process' }
if (!$Cli) { $Cli = Join-Path $repo 'target/debug/envbox.exe' }
if (!$Probe64) { $Probe64 = Join-Path $repo 'target/debug/envbox-probe.exe' }
if (!$Probe32) { $Probe32 = Join-Path $repo 'target/i686-pc-windows-msvc/debug/envbox-probe.exe' }
foreach ($file in @($Cli,$Probe64,$Probe32,(Join-Path $RuntimeDirectory 'envbox-runtime64.dll'),(Join-Path $RuntimeDirectory 'envbox-runtime32.dll'))) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing fixture artifact: $file" }
}
Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
$output = Join-Path $repo "target/$Prefix-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $output | Out-Null
$env:ENVBOX_CONFIG_ROOT = Join-Path $output 'config'
function Invoke-Cli([string[]]$Arguments) {
    $ErrorActionPreference = 'Continue' # Windows PowerShell treats native stderr as ErrorRecord.
    $result = & $Cli @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "CLI failed: $($result -join [Environment]::NewLine)" }
}
function Read-Probe([string]$Path, [string]$Profile = '', [string]$Child = '') {
    $ErrorActionPreference = 'Continue'
    $argsProbe = @('--identity-json')
    if ($Child) { $argsProbe += @('--identity-child',$Child) }
    if ($Profile) {
        $env:ENVBOX_RUNTIME_DLL = Join-Path $RuntimeDirectory $(if ($Path -eq $Probe32) {'envbox-runtime32.dll'} else {'envbox-runtime64.dll'})
        # stderr carries CLI lifecycle logs; only stdout is the Probe JSON stream.
        $text = & $Cli run --profile $Profile -- $Path @argsProbe 2> (Join-Path $output 'last-run.stderr.log')
    } else {
        Remove-Item Env:ENVBOX_RUNTIME_DLL -ErrorAction SilentlyContinue
        $text = & $Path @argsProbe
    }
    if ($LASTEXITCODE -ne 0) { throw 'Probe launch failed; see last-run.stderr.log' }
    @($text | Where-Object { $_.StartsWith('{') } | ForEach-Object { $_ | ConvertFrom-Json })
}
function Assert-Identity($Value, [string]$Computer, [string]$User, [string]$Mac, [string]$Guid) {
    if ($Value.RuntimeLoaded -ne 'true') { throw 'Profile probe did not load Runtime' }
    if ($Value.GetHostNameW_BeforeStartup -ne 'status=-1;error=10093') { throw 'Winsock initialization contract failed for DNS Host identity fixture' }
    foreach ($api in @('gethostname','GetHostNameW')) {
        if ($Value.$api -cne $Computer) { throw "$api differs from Profile" }
        if ($Value."${api}_ShortContract" -ne 'status=-1;error=10014' -or $Value."${api}_NullContract" -ne 'status=-1;error=10014') { throw "$api Winsock buffer contract failed" }
    }
    foreach ($suffix in @('A','W')) {
        foreach ($pair in @(@('GetComputerName',$Computer,111),@('GetUserName',$User,122))) {
            $api = "$($pair[0])$suffix"
            if ($Value.$api -cne $pair[1]) { throw "$api differs from Profile" }
            if ($Value."${api}_ShortContract" -ne "ok=0;error=$($pair[2]);required=$($pair[1].Length+1)") { throw "$api short-buffer contract failed" }
            $expectedSize = $pair[1].Length + $(if ($pair[0] -eq 'GetUserName') {1} else {0})
            if ([int]$Value."${api}_SuccessSize" -ne $expectedSize) { throw "$api success size failed" }
        }
        foreach ($format in 0..7) {
            $expected = $(if ($format -in @(2,6)) {''} else {$Computer})
            $api = "GetComputerNameEx${suffix}_$format"
            if ($Value.$api -cne $expected) { throw "$api differs from Profile" }
            if ($Value."${api}_ShortContract" -ne "ok=0;error=234;required=$($expected.Length+1)") { throw "$api short-buffer contract failed" }
        }
    }
    foreach ($api in @('GetAdaptersAddresses','GetAdaptersInfo','GetIfTable','GetIfEntry','GetIfTable2','GetIfEntry2')) {
        $observed = $Value."${api}_MAC"
        if ($observed -cne $Mac) { throw "$api six-byte MAC view differs from Profile: $observed" }
    }
    if ($Value.GetIfTable2_PermanentMAC -cne $Mac) { throw 'Permanent MAC view differs' }
    foreach ($property in $Value.PSObject.Properties | Where-Object { $_.Name -match '^MachineGuid_.*_(0|256|512)$' }) {
        # A redirected registry key can be absent in one WOW64 view. Its open
        # error is retained as evidence; successful handles must all show Profile.
        if ($property.Value -like 'open_status=*') { continue }
        if ($property.Value -cne $Guid) { throw "$($property.Name) differs from Profile" }
        $contract = $Value."$($property.Name)_Contract"
        $bytes = ($Guid.Length+1) * $(if ($property.Name -match 'W_') {2} else {1})
        if ($contract -ne "type=1;size_status=0;required=$bytes;short_status=234;short_required=$bytes") { throw "$($property.Name) registry buffer contract failed: $contract" }
        if ($property.Name -match 'RegGetValue' -and $Value."$($property.Name)_ZeroOnFailure" -cne "status=234;required=$bytes;zeroed=true;canary=true") { throw "$($property.Name) ZEROONFAILURE capacity guard failed" }
    }
    if ($Value.Environment_COMPUTERNAME -cne $Computer -or $Value.Environment_USERNAME -cne $User) { throw 'Identity environment does not match API view' }
}
$common = @('--locale','en-US','--ui-language','en-US','--region','US','--tz-windows','UTC','--tz-iana','Etc/UTC','--dns-mode','host')
$a = @('AURA-A','profile_a','02:AA:00:00:00:01','11111111-1111-4111-8111-111111111111')
$b = @('AURA-B','profile_b','02:BB:00:00:00:02','22222222-2222-4222-8222-222222222222')
Invoke-Cli (@('profile','add','--name','identity-default')+$common)
foreach ($entry in @(@('identity-a',$a),@('identity-b',$b))) {
    $v = $entry[1]
    Invoke-Cli (@('profile','add','--name',$entry[0])+$common+@('--computer-name',$v[0],'--user-name',$v[1],'--mac-address',$v[2],'--machine-guid',$v[3]))
}
$before64 = (Read-Probe $Probe64)[0]
$before32 = (Read-Probe $Probe32)[0]
if ($before64.RuntimeLoaded -ne 'false' -or $before32.RuntimeLoaded -ne 'false') { throw 'Host baseline loaded Runtime' }
$cases = @()
foreach ($probe in @($Probe64,$Probe32)) {
    $baseline = $(if ($probe -eq $Probe64) {$before64} else {$before32})
    $default = (Read-Probe $probe 'identity-default')[0]
    foreach ($property in $baseline.PSObject.Properties | Where-Object Name -NE 'RuntimeLoaded') {
        if ($default.($property.Name) -cne $property.Value) { throw "Disabled identity changed $($property.Name)" }
    }
    foreach ($entry in @(@('identity-a',$a),@('identity-b',$b),@('identity-a',$a))) {
        $values = Read-Probe $probe $entry[0] $probe
        if ($values.Count -ne 2) { throw 'Expected parent and child identity snapshots' }
        $expected = $entry[1]
        foreach ($value in $values) { Assert-Identity $value $expected[0] $expected[1] $expected[2] $expected[3] }
        $cases += [pscustomobject]@{ Architecture = $(if ($probe -eq $Probe64) {'x64'} else {'x86'}); Profile=$entry[0]; ParentAndChild=$values }
    }
}
$mixed = @(@($Probe64,$Probe32,'x64-to-x86'),@($Probe32,$Probe64,'x86-to-x64'))
foreach ($entry in $mixed) {
    $values = Read-Probe $entry[0] 'identity-a' $entry[1]
    if ($values.Count -ne 2) { throw 'Expected mixed-architecture parent and child snapshots' }
    foreach ($value in $values) { Assert-Identity $value $a[0] $a[1] $a[2] $a[3] }
    $cases += [pscustomobject]@{ Architecture=$entry[2]; Profile='identity-a'; ParentAndChild=$values }
}
$after64 = (Read-Probe $Probe64)[0]
$after32 = (Read-Probe $Probe32)[0]
if (($before64 | ConvertTo-Json -Compress) -cne ($after64 | ConvertTo-Json -Compress) -or ($before32 | ConvertTo-Json -Compress) -cne ($after32 | ConvertTo-Json -Compress)) { throw 'Host identity changed during fixture' }
$result = [pscustomobject]@{ ControllerPid=$PID; RuntimeModules=$loaded.Count; Passed=$true; Cases=$cases; HostUnchanged=$true; DisabledIdentityUnchanged=$true }
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'result.json') -Encoding UTF8
Write-Output "IDENTITY_ACCEPTANCE_OK $output"
