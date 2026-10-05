# Native DoH acceptance probe

This probe links the product-default `envbox-dns-doh` static library (`--no-default-features`) into a small MSVC DLL and invokes the public C ABI from an independent executable. It uses the real Windows ROOT/CA/Disallowed stores and the real local CRL/cache snapshot. It does not install certificates, use the fixture trust feature, change the host DNS configuration, enable a proxy, or require elevation.

The transport receives four explicit URL/bootstrap pairs: Cloudflare at `1.1.1.1` and `2606:4700:4700::1111`, and Google at `8.8.8.8` and `2001:4860:4860::8888`. It sends a minimal `example.com A IN` DNS query and records the typed error, returned length, message ID/QR/RCODE, and a complete wire-question comparison (case-insensitive QNAME plus exact QTYPE and QCLASS). IPv6 unavailability and native trust failure remain recorded outcomes; neither is silently promoted to a pass.

Run from the repository root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/run.ps1
Get-Content target/doh-native-acceptance-results.json -Raw
```

Every invocation creates a fresh `run_id` and waits up to three minutes for
that WMI worker's uniquely named result. The stable result/log paths above are
copied only after the matching run has completed. A completed harness with
typed certificate/network errors is recorded with `completed=true`,
`harness_completed=true`, `gate=false`, and the top-level script exits nonzero
when the positive acceptance gate is closed; it never treats an old result or
an asynchronously launched worker as this run's pass.

`-RunId` is an internal WMI-worker handoff parameter. Top-level invocations
must omit it and always receive a newly generated GUID; a worker accepts only a
valid 32-hex-digit GUID in `N` format. Before starting build/WMI work, each
wrapper also refuses to reuse an existing per-run result file.

To capture the inner Rustls certificate error while reusing the production
`Snapshot::load()` and `Snapshot::verifier()` source directly, run the isolated
diagnostic package. It performs only a literal TCP/TLS handshake, sends no
HTTP, and still uses the production verifier; it does not install a trust
anchor or use an accept-all verifier:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/diagnostic/run.ps1
Get-Content target/doh-native-diagnostic-results.json -Raw
Get-Content target/doh-native-diagnostic-64.log -Raw
Get-Content target/doh-native-diagnostic-32.log -Raw
```

The diagnostic has the same per-run WMI wait and result marker. Its
`gate=false` is intentional: typed `UnknownIssuer`, `UnknownRevocationStatus`,
network, and timeout observations are completed diagnostic evidence, so a
completed worker may exit zero while the JSON still cannot claim acceptance.
Literal TCP connect and Rustls TLS handshake each have a 15-second endpoint
deadline inside a 60-second worker budget.

The diagnostic also records presented-chain CDP cache-only observations:
candidate CRL count, byte length and SHA-256, marked `authenticated=0`. It uses
the production read-only cache adapter with fixed cache-only/no-write flags;
these observations neither inject trust nor prove full chain revocation.
The product verifier independently performs any permitted strict cache retry.
Fixture trust remains self-contained and does not read the host URL cache.
See the [cache slice evidence](../../.scratch/aura-container/evidence/doh-offline-crl-cache.md).

For research only, setting `DOH_DIAGNOSTIC_INCLUDE_AUTHROOT=1` before the
diagnostic build/run creates an isolated diagnostic variant that includes the
current physical AuthRoot store as candidate anchors. This switch changes only
generated diagnostic source under a separate target directory; it never changes
the product crate or the Windows trust store. Its purpose is to expose the next
strict-policy failure after `UnknownIssuer`, not to provide a production trust
configuration. Remove the variable after the run.

To view the public peer chains independently of Rustls, the optional
`capture-chain.ps1` uses direct literal-IP Schannel connections and rejects the
certificate after recording the Windows-built subject/issuer chain. It never
loads the product DLL, changes trust, or treats Schannel acceptance as a
product pass. The Rustls diagnostic logs the raw leaf/intermediate chain before
delegating to the production verifier:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/capture-chain.ps1
Get-Content target/doh-native-chain-capture.log -Raw
```

The capture worker uses unique run/result files and a 15-second per-endpoint
TCP/TLS deadline inside a 60-second total budget. The Schannel callback is a
small native C# callback so it can run on the TLS worker thread; it records the
peer chain and always rejects it. If a handshake is ever
`accepted_unexpectedly`, the result is marked invalid and the worker/top-level
script exits nonzero.

The script builds x64 and Win32/i686 product static libraries in an isolated target directory, checks that fixture symbols are absent, builds the native DLL/host, then uses `Win32_Process.Create` to execute a fresh hidden WMI worker. The worker clears inherited `ENVBOX_*` variables and verifies that no `envbox-runtime*` module is loaded. The raw per-architecture logs are `target/doh-native-acceptance-{64,32}.log`.

This is intentionally a positive/negative native trust probe, not a global packet capture. A successful response demonstrates the configured literal endpoint, the URL identity and the default offline trust snapshot for this Windows installation. A typed `revocation_unknown`, `trust_snapshot`, `certificate` or `network` result is evidence of the precise production blocker. The result does not prove that every Host DNS/PAC/Cryptnet/AFD path is absent, nor does one Windows installation establish an OS-version matrix. Those observations remain required before enabling product DoH.
