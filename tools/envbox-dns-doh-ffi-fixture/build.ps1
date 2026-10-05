$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
$cmake = 'D:/Tools/VS2022BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe'
$dumpbin = 'D:/Tools/VS2022BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64/dumpbin.exe'
$metadata = (& cargo metadata --format-version 1 --locked --offline | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'Cargo dependency metadata failed' }
foreach ($item in @(@{Bits='64';Rust='x86_64-pc-windows-msvc';VS='x64'},@{Bits='32';Rust='i686-pc-windows-msvc';VS='Win32'})) {
    $build = Join-Path $repo "target/doh-native-ffi$($item.Bits)"
    New-Item -ItemType Directory -Path $build -Force | Out-Null
    & cargo rustc -p envbox-dns-doh --lib --target $item.Rust --target-dir target/doh-product-prototype --no-default-features --locked -- --print native-static-libs *> "$build/rust-staticlib.log"
    if ($LASTEXITCODE -ne 0) { throw "Rust staticlib failed: $build/rust-staticlib.log" }
    $line = Get-Content "$build/rust-staticlib.log" | Where-Object { $_ -match 'native-static-libs:' } | Select-Object -Last 1
    if (!$line) { throw 'Cargo native-static-libs output missing' }
    $native = @(($line -replace '^.*native-static-libs:\s*','') -split '\s+' | Where-Object { $_ })
    $packageName = if ($item.Bits -eq '64') { 'windows_x86_64_msvc' } else { 'windows_i686_msvc' }
    $importPackage = $metadata.packages | Where-Object { $_.name -eq $packageName -and $_.version -eq '0.52.6' } | Select-Object -First 1
    if (!$importPackage) { throw 'Pinned windows-targets import package not found' }
    $windowsImport = Join-Path (Split-Path $importPackage.manifest_path) 'lib/windows.0.52.0.lib'
    $libraries = ($native | Where-Object { ! $_.StartsWith('/') } | ForEach-Object { if ($_ -eq 'windows.0.52.0.lib') { $windowsImport } else { $_ } }) -join ';'
    $options = ($native | Where-Object { $_.StartsWith('/') }) -join ';'
    $staticlib = Join-Path $repo "target/doh-product-prototype/$($item.Rust)/debug/envbox_dns_doh.lib"
    & $dumpbin /linkermember:1 $staticlib *> "$build/rust-symbols.log"
    if ($LASTEXITCODE -ne 0) { throw 'Staticlib symbol scan failed' }
    if (Select-String -LiteralPath "$build/rust-symbols.log" -Pattern 'query_fixture|Snapshot7fixture|from_fixture_material|fixture_snapshot') { throw 'Fixture trust symbol in product staticlib' }
    & $cmake -S $PSScriptRoot -B $build -A $item.VS "-DDOH_STATICLIB=$staticlib" "-DDOH_NATIVE_LIBS=$libraries" "-DDOH_NATIVE_OPTIONS=$options" *> "$build/configure.log"
    if ($LASTEXITCODE -ne 0) { throw "MSVC configure failed: $build/configure.log" }
    & $cmake --build $build --config Release *> "$build/build.log"
    if ($LASTEXITCODE -ne 0) { throw "MSVC link failed: $build/build.log" }
    & $dumpbin /exports /imports "$build/Release/doh-ffi-smoke.dll" *> "$build/dll-pe.log"
    if ($LASTEXITCODE -ne 0) { throw 'PE inspection failed' }
}
