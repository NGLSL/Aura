# Native DoH FFI smoke

This standalone fixture links the product Rust `staticlib` into an MSVC DLL,
then loads that DLL from a native EXE and invokes the public C header ABI.
It does not enable Aura Runtime DoH capability or use `fixture-trust`.

Run `build.ps1` from PowerShell. It builds `--no-default-features --locked`
static libraries for x86_64 and i686 in `target/doh-product-prototype`, obtains
the native link libraries from Cargo, resolves the matching windows-targets
import archive through locked Cargo metadata, and links with the DLL CRT
(`/MD`, matching Rust's reported `/defaultlib:msvcrt`). Build and PE inspection
logs are in `target/doh-native-ffi64` and `target/doh-native-ffi32`.

Run `run.ps1` in an uninjected process. From an injected development shell,
use a fresh hidden WMI process:

```powershell
Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
    CommandLine='pwsh.exe -NoProfile -WindowStyle Hidden -File D:\Project\Aura\tools\envbox-dns-doh-ffi-fixture\run.ps1'
}
```

The DLL's `DllMain` makes no Rust call. The host invokes the smoke function
after `LoadLibrary` returns. Both architectures check invalid arguments
(error 1), cancellation with the native cdecl callback (error 2, return -1),
an already expired deadline (error 3), and the product offline trust path.
The final call uses only `https://aura-doh-ffi.test:<port>/dns-query` with explicit
`127.0.0.1`. The native host reserves an exclusive, bound, non-listening ephemeral
loopback port and retains it until the call returns; another service cannot
claim that endpoint. No hostname resolver or public endpoint is requested. A
trust snapshot failure, a closed loopback socket, or bounded deadline are
accepted failures. In the initial native run, both architectures returned
error 11 (`TrustSnapshot`); this proves error propagation, without identifying
which snapshot read or validation failed. It does not establish missing CRL
material (error 7 would represent revocation status unknown).
After the trust adapter fix, the final native run returned error 4 (`Network`)
on both architectures at the fixture-owned non-listening endpoint.
No trust store is changed. A successful smoke establishes
link/load/calling-convention/error-classification compatibility; it does not
establish a trusted positive DoH exchange or production readiness.
