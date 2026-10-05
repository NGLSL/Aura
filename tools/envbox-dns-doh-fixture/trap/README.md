# DoH fixture process API and endpoint trap

This is test instrumentation, not a product Runtime or a security boundary. It
modifies only the fixture process that explicitly loads this DLL. The backend
must be constructed after installation; keep the DLL loaded until process exit.

Build each architecture with CMake/MSVC and the existing Detours installation:

```powershell
cmake -S tools/envbox-dns-doh-fixture/trap -B target/doh-api-trap64 -A x64
cmake --build target/doh-api-trap64 --config Release
cmake -S tools/envbox-dns-doh-fixture/trap -B target/doh-api-trap32 -A Win32
cmake --build target/doh-api-trap32 --config Release
```

The exports have the same undecorated names on x64 and x86. See `api-trap.h`:

1. Explicitly `LoadLibraryW` the architecture-matched DLL.
2. Call `DoHApiTrapAllowEndpoint(literal_ip, host_order_port)`. This performs no
   name resolution. Exactly one TCP destination is allowed.
3. Call `DoHApiTrapInstall()` and require return zero. Missing mandatory exports
   or an attach/commit failure invalidates the observation; optional missing Raw
   and newer proxy exports appear as unavailable in the snapshot.
4. Construct and execute the backend, then call `DoHApiTrapSnapshot` with a 16 KiB
   writable buffer and require return zero. Retain the DLL through process exit.

For a clean backend run, all 18 DNS, WinHTTP/PAC, and CryptoAPI certificate-chain
API counters must remain zero. `sendto` and `WSASendTo` must remain zero.
`connects_denied` and `extension_denied` must remain zero, while
`connects_allowed` must be positive. Connection API counters are expected to be
positive; they identify the actual path used. The trap enforces SOCK_STREAM,
literal address, address family, IPv6 scope, and exact port before forwarding.

ConnectEx requires special handling: WSAIoctl normally returns a provider
function pointer. The trap replaces that pointer with its endpoint-checking
wrapper, remembers the original separately by provider family, and refuses an
unrecognized replacement. Asynchronous extension lookup and other extension
function pointers, including WSASendMsg/RIO paths, are refused. The ordinary
WSAIoctl operations used for IOCP are forwarded. A successful install alone does
not prove ConnectEx interception: the native check obtains and calls the actual
extension pointer and verifies the peer of an accepted loopback connection.

`DoHApiTrapSelfTest()` intentionally invokes blocked getaddrinfo, WinHttpOpen,
and CertGetCertificateChain. Run it in a separate process from the clean backend
sample. The native `envbox-doh-api-trap-check.exe` additionally verifies one
allowed ConnectEx connection, denied connect/WSAConnect destinations, denied
UDP sendto/WSASendTo, and refusal to hand out WSASendMsg. `run-host.ps1` executes
both native architectures and writes logs plus hashes under `target`; launch
that script using the project's WMI fresh Host pattern because ordinary agent
descendants may already be injected.

## Observation boundaries

Pair the trap with the worker-owned TLS/H2 server fixture. Record the server's
listening endpoint and peer, TLS SNI and selected ALPN, HTTP/2 `:authority`,
content type and request path. Give that certificate AIA/CRL/OCSP URLs pointing
at separate local canary listener ports, not at the allowed TLS endpoint; count
actual canary connections and bytes. Canary silence together with zero blocked
API/endpoint counters describes this backend invocation. A denied trap count is
an attempted auxiliary operation, even when no packet reaches a canary, and must
fail the gate. Test invalid identity, untrusted/expired/revoked certificates and
confirm DNS application bytes are not accepted before verification.

On this unelevated machine the fresh Host preflight actually found own-PID
`Get-NetTCPConnection` usable, DNS-Client transient ETW session start denied with
0x80070005, no matching readable DNS-Client Operational event, and no usable
Get-NetEventSession observation. The preflight never enabled event channels or
changed trust, DNS or proxy settings. ProcessStopTrace was separately denied in
the Git diagnostics. Own-PID TCP snapshots can miss short-lived connections;
they are supporting endpoint evidence, not packet capture.

This trap does not inspect direct native AFD/NT calls, sockets created before
installation, direct invocation of cached extension pointers, other processes,
or traffic from a system service. It is not a WFP/ETW capture or proof of zero
DNS across the machine. Cache callbacks and HTTP server canary counters do not
provide that wider proof. State the process, install boundary, API availability,
positive tripwire results, actual socket path, and these limits with any result.
