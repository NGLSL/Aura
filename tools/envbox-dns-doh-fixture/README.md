# DoH standalone fixture

The binary is opt-in and uses only generated fixture trust. Default workspace
builds do not enable it. Its target directory must remain separate from the
product staticlib target directory.

```powershell
cargo build -p envbox-dns-doh-fixture --features fixture-trust `
  --target-dir target/doh-fixture-prototype --locked
cargo build -p envbox-dns-doh-fixture --features fixture-trust `
  --target i686-pc-windows-msvc --target-dir target/doh-fixture-prototype --locked
```

`run.py` requires cryptography and Python h2. Use a project-local virtual
environment with an explicit pip `--target` to its own site-packages; this
machine's pip configuration can redirect even venv installation to a global
directory. Do not install or upgrade global packages. Each run creates a UUID
directory under `target` for private fixture keys/certificates/CRLs.

Build the architecture-matched own-process instrumentation from `trap/`, then:

```powershell
python tools/envbox-dns-doh-fixture/run.py `
  target/doh-fixture-prototype/debug/envbox-dns-doh-fixture.exe `
  target/doh-api-trap64/Release/envbox-doh-api-trap.dll `
  target/doh-fixture-prototype/i686-pc-windows-msvc/debug/envbox-dns-doh-fixture.exe `
  target/doh-api-trap32/Release/envbox-doh-api-trap.dll
```

The server uses mature Python h2, with TLS over a dynamic loopback endpoint. It
records peer IP, SNI, ALPN, TLS version and HTTP authority. AIA/CRL URLs point at
a separate local canary listener. No certificates enter Windows trust stores.
The native fixture loads its own trap explicitly, configures the permitted
endpoint and installs hooks before constructing the backend. It prints a JSON
snapshot after return. Read `trap/README.md` for instrumentation boundaries.

The matrix covers h2 TLS 1.3 and forced TLS 1.2, h1, 16 sequential queries and
handle cleanup, 65535-byte bodies and streamed overflow, untrusted/expired/name
errors, root EKU, full chain revocation, unknown/stale/wrong-issuer CRLs,
EE/CA explicit denial, non-2xx/redirect/media/encoding errors, handshake/read
timeout and cancellation. Certificate/revocation failures must precede HTTP
application requests. Forbidden API counts, denied endpoints, UDP calls and
canary traffic must remain zero.

The matrix also verifies an IPv4 IP URL against its IP SAN, with no DNS SNI.
Five IPv6 cases per architecture cover hostname URLs over an IPv6 bootstrap,
HTTP/1.1 and HTTP/2, a bracketed IP URL with IP SAN validation, wrong IP
identity, and read cancellation. A plain-socket `::1` preflight runs first.
If the host denies IPv6, the runner explicitly reports those cases as
UNVERIFIED and does not count them as passed; the executed IPv4 matrix still
runs. Fixing host networking is outside this fixture's scope.

Fresh hidden WMI runs provide a process environment independent from an injected
interactive shell. These are process-scoped instrumentation and loopback
observations, not global ETW/packet capture or a complete Windows-version matrix.
Fixture trust success does not demonstrate a public resolver is usable with the
machine's production ROOT/CRL snapshot. The separate native FFI fixture proves
default-feature-off Rust staticlibs link with MSVC C++ on both architectures.

The matrix now also supplies explicit `--cached-crls` DER candidates through
the fixture-only trust input. Thirteen cases per architecture exercise the
production retry and standard verifier branch: valid/stale-refresh positives,
miss, wrong issuer, expired or tampered-signature CRLs, revoked EE/CA, full chain
coverage, unknown CA revocation, inability to override known revoked, wrong
name and untrusted chain. No Windows cache entry is seeded or read. This adds
26 cases to the previous 58 executed IPv4 cases, for 84 when both architectures
run; the 10 optional IPv6 cases remain separately qualified by preflight.

Four further cases per architecture use `--native-cache true` to exercise the
actual Windows CDP extraction and cache-only CRL collector with fixture roots.
The randomly allocated loopback CDP has no seeded cache entry. A plain miss must
return revocation-unknown before HTTP. `--cancel-cache-check` emits a single
caller-thread cancellation pulse at cache check 1 (collector entry), 5 (after
the first CDP CAPI call), or 10 (after cache-only retrieval). The query must
return Cancelled even though the callback returns zero again after the query;
the output records exact cache check and pulse counts. These cases require zero
HTTP requests, canary connections, forbidden host API calls, and denied endpoint
attempts. No cache or trust material is written. Both architectures now execute
92 IPv4 cases; the 10 IPv6 cases still require the existing preflight.

`--tls-revocation standard|strict_offline` selects the fixture TLS revocation
policy; omission retains `strict_offline` for all original 92 cases. Standard
TLS adds ten cases per architecture: missing leaf/CA revocation material may
succeed, while known EE/CA revocation, untrusted issuer, wrong identity, expired
certificate, invalid certificate signature, and explicit denial still fail.
Selecting standard after `--native-cache true` must disable cache collection
(`cache_checks=0`) while the valid fixture makes one HTTP request. All existing
forbidden API, UDP, denied endpoint, and canary assertions remain active. An
invalid policy is rejected before loading instrumentation or starting TLS.
There are 112 IPv4 TLS cases plus two CLI rejection cases across both
architectures; the optional ten IPv6 cases remain separately qualified.
