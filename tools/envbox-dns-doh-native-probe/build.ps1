$ErrorActionPreference = 'Stop'
# Cargo/CMake write normal progress to stderr. Native exit codes are checked
# explicitly below, so do not turn successful progress output into exceptions.
$PSNativeCommandUseErrorActionPreference = $false
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $repo
$cmake = 'D:/Tools/VS2022BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe'
$dumpbin = 'D:/Tools/VS2022BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64/dumpbin.exe'

function Invoke-NativeExitCode {
    param([scriptblock]$Command)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    return $code
}

$metadataPrevious = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    $metadataJson = & cargo metadata --format-version 1 --locked --offline
    $metadataCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $metadataPrevious
}
if ($metadataCode -ne 0) { throw 'Cargo dependency metadata failed' }
$metadata = $metadataJson | ConvertFrom-Json

foreach ($item in @(@{Bits='64'; Rust='x86_64-pc-windows-msvc'; VS='x64'}, @{Bits='32'; Rust='i686-pc-windows-msvc'; VS='Win32'})) {
    $build = Join-Path $repo "target/doh-native-acceptance$($item.Bits)"
    New-Item -ItemType Directory -Path $build -Force | Out-Null
    # Start-Process keeps rustc's long native-static-libs line intact under
    # Windows PowerShell 5.1; invoking cargo directly there wraps stderr into
    # an ErrorRecord and can split `dbghelp.lib` while parsing the line.
    $stdoutLog = Join-Path $build 'rust-static-libs.stdout.log'
    $stderrLog = Join-Path $build 'rust-static-libs.stderr.log'
    $cargo = Start-Process -FilePath 'cargo.exe' -ArgumentList @(
        '+1.99.0', 'rustc', '-p', 'envbox-dns-doh', '--lib', '--target', $item.Rust,
        '--target-dir', 'target/doh-native-acceptance-product', '--no-default-features',
        '--locked', '--', '--print', 'native-static-libs'
    ) -NoNewWindow -Wait -PassThru -RedirectStandardOutput $stdoutLog -RedirectStandardError $stderrLog
    $code = $cargo.ExitCode
    $cargoOutput = @(Get-Content -LiteralPath $stdoutLog -ErrorAction SilentlyContinue) + @(Get-Content -LiteralPath $stderrLog -ErrorAction SilentlyContinue)
    $cargoOutput | Set-Content -LiteralPath (Join-Path $build 'rust-static-libs.log') -Encoding utf8
    if ($code -ne 0) { throw "Rust staticlib build failed: $build/rust-static-libs.log (exit=$code)" }
    $line = $cargoOutput | Where-Object { $_ -match 'native-static-libs:' } | Select-Object -Last 1
    if (!$line) { throw 'native-static-libs output missing' }
    $native = @(($line -replace '^.*native-static-libs:\s*','') -split '\s+' | Where-Object { $_ })
    $packageName = if ($item.Bits -eq '64') { 'windows_x86_64_msvc' } else { 'windows_i686_msvc' }
    $importPackage = $metadata.packages | Where-Object { $_.name -eq $packageName -and $_.version -eq '0.52.6' } | Select-Object -First 1
    if (!$importPackage) { throw 'Pinned windows-targets package not found' }
    $windowsImport = Join-Path (Split-Path $importPackage.manifest_path) 'lib/windows.0.52.0.lib'
    $libraries = ($native | Where-Object { ! $_.StartsWith('/') } | ForEach-Object { if ($_ -eq 'windows.0.52.0.lib') { $windowsImport } else { $_ } }) -join ';'
    $options = ($native | Where-Object { $_.StartsWith('/') }) -join ';'
    $staticlib = Join-Path $repo "target/doh-native-acceptance-product/$($item.Rust)/debug/envbox_dns_doh.lib"
    $code = Invoke-NativeExitCode {
        & $dumpbin /linkermember:1 $staticlib *> "$build/rust-symbols.log"
    }
    if ($code -ne 0) { throw 'Product staticlib symbol scan failed' }
    if (Select-String -LiteralPath "$build/rust-symbols.log" -Pattern 'query_fixture|Snapshot7fixture|from_fixture_material|fixture_snapshot') { throw 'Fixture trust symbol leaked into product staticlib' }
    $code = Invoke-NativeExitCode {
        & $cmake -S $PSScriptRoot -B $build -A $item.VS "-DDOH_STATICLIB=$staticlib" "-DDOH_NATIVE_LIBS=$libraries" "-DDOH_NATIVE_OPTIONS=$options" *> "$build/configure.log"
    }
    if ($code -ne 0) { throw "CMake configure failed: $build/configure.log (exit=$code)" }
    $code = Invoke-NativeExitCode {
        & $cmake --build $build --config Release *> "$build/build.log"
    }
    if ($code -ne 0) { throw "CMake build failed: $build/build.log (exit=$code)" }
    $code = Invoke-NativeExitCode {
        & $dumpbin /exports /imports "$build/Release/doh-native-probe.dll" *> "$build/dll-pe.log"
    }
    if ($code -ne 0) { throw 'PE inspection failed' }
}
