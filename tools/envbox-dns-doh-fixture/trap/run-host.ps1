Set-Location -LiteralPath 'D:\Project\Aura'
Get-ChildItem Env:ENVBOX* | ForEach-Object { Remove-Item -LiteralPath ('Env:' + $_.Name) }
if (@((Get-Process -Id $PID).Modules | Where-Object { $_.ModuleName -like 'envbox-runtime*' }).Count -ne 0) { throw 'trap control process injected' }
$results=@()
foreach($architecture in @('64','32')) {
 $directory='D:\Project\Aura\target\doh-api-trap'+$architecture+'\Release'
 & ($directory+'\envbox-doh-api-trap-check.exe') ($directory+'\envbox-doh-api-trap.dll') *> ('target/doh-api-trap-check'+$architecture+'.log')
 $results+=@{architecture=$architecture;exit=$LASTEXITCODE;dll_sha256=(Get-FileHash -LiteralPath ($directory+'\envbox-doh-api-trap.dll')).Hash}
}
@{host_runtime_modules=0;results=$results} | ConvertTo-Json -Depth 3 | Set-Content target/doh-api-trap-check-results.json
if (@($results|Where-Object {$_.exit -ne 0}).Count -ne 0) { exit 1 }
