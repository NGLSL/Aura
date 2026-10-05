$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
$runtime = @((Get-Process -Id $PID).Modules | Where-Object ModuleName -Like 'envbox-runtime*')
"host_pid=$PID runtime_modules=$($runtime.Count)" | Set-Content target/doh-native-ffi-host.log
if ($runtime.Count -ne 0) { throw 'Host runner is injected; launch through fresh WMI process' }
Get-ChildItem Env: | Where-Object Name -Like 'ENVBOX_*' | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
foreach ($bits in @('64','32')) {
    $build = Join-Path $repo "target/doh-native-ffi$bits"
    & "$build/Release/doh-ffi-host.exe" "$build/Release/doh-ffi-smoke.dll" *> "$build/smoke.log"
    $code = $LASTEXITCODE
    "arch=$bits exit=$code" | Add-Content target/doh-native-ffi-host.log
    if ($code -ne 0) { throw "Native smoke failed: $build/smoke.log" }
}
