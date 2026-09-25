# Hot-path timing for EnvBox Runtime hooks (Chrome-like load).
# Usage: .\scripts\bench-hooks.ps1
param(
    [string]$Profile = "US Development",
    [int]$DnsN = 30,
    [int]$ApiN = 2000,
    [int]$ProcN = 8
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$envbox = Join-Path $root "target\debug\envbox.exe"
if (-not (Test-Path $envbox)) { throw "missing $envbox" }

$innerPath = Join-Path $env:TEMP "envbox-bench-inner.ps1"
$inner = @'
param([int]$DnsN, [int]$ApiN, [int]$ProcN)

function Time-It([string]$label, [scriptblock]$body) {
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  & $body
  $sw.Stop()
  Write-Output ("{0} ms={1}" -f $label, [int]$sw.Elapsed.TotalMilliseconds)
}

Time-It "dns-unique" {
  $ok = 0
  for ($i = 0; $i -lt $DnsN; $i++) {
    $n = "www-$i.bing.com"
    try {
      $a = [System.Net.Dns]::GetHostAddresses($n)
      if ($a) { $ok++ }
    } catch {}
  }
  Write-Output ("dns-unique ok={0}" -f $ok)
}

Time-It "dns-repeat" {
  $ok = 0
  for ($i = 0; $i -lt $DnsN; $i++) {
    try {
      $a = [System.Net.Dns]::GetHostAddresses("www.bing.com")
      if ($a) { $ok++ }
    } catch {}
  }
  Write-Output ("dns-repeat ok={0}" -f $ok)
}

Time-It "tz-api" {
  $sig = '[DllImport("kernel32.dll")] public static extern uint GetTimeZoneInformation(System.IntPtr ptzi);'
  try {
    Add-Type -Namespace EnvBoxBench -Name Tz -MemberDefinition $sig | Out-Null
  } catch {}
  $buf = [Runtime.InteropServices.Marshal]::AllocHGlobal(172)
  for ($i = 0; $i -lt $ApiN; $i++) {
    [void][EnvBoxBench.Tz]::GetTimeZoneInformation($buf)
  }
  [Runtime.InteropServices.Marshal]::FreeHGlobal($buf)
}

Time-It "spawn" {
  for ($i = 0; $i -lt $ProcN; $i++) {
    Start-Process -FilePath "cmd.exe" -ArgumentList "/c","exit 0" -WindowStyle Hidden -Wait
  }
}
'@
Set-Content -Path $innerPath -Value $inner -Encoding UTF8

function Invoke-Bench([string]$title, [string[]]$prefix) {
    Write-Host ("== {0} ==" -f $title)
    $argv = New-Object System.Collections.Generic.List[string]
    if ($prefix) { foreach ($p in $prefix) { $argv.Add($p) } }
    $argv.Add("powershell")
    $argv.Add("-NoProfile")
    $argv.Add("-ExecutionPolicy")
    $argv.Add("Bypass")
    $argv.Add("-File")
    $argv.Add($innerPath)
    $argv.Add("-DnsN")
    $argv.Add("$DnsN")
    $argv.Add("-ApiN")
    $argv.Add("$ApiN")
    $argv.Add("-ProcN")
    $argv.Add("$ProcN")

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $out = & $argv[0] @($argv | Select-Object -Skip 1)
    $sw.Stop()
    if ($out) { $out | ForEach-Object { Write-Host ("  {0}" -f $_) } }
    Write-Host ("  wall={0}ms" -f [int]$sw.Elapsed.TotalMilliseconds)
}

Invoke-Bench "baseline (no injection)" @()
Invoke-Bench "injected (no audit)" @($envbox, "run", "--profile", $Profile, "--")
Invoke-Bench "injected + audit" @($envbox, "run", "--profile", $Profile, "--audit", "--")
