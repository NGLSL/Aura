# Native DoH acceptance probe

This probe links the product-default `envbox-dns-doh` static library (`--no-default-features`) into a small MSVC DLL and invokes `envbox_doh_query_with_policy` from independent executables. Trust combines the pinned public root bundle with the read-only Windows ROOT/CA/Disallowed snapshot. Standard TLS checks chain/name/time/signatures and available CRLs without requiring complete offline revocation coverage. StrictOffline additionally requires known fresh revocation status and permits cache-only CRL lookup. No certificates are installed, no fixture trust is linked, and no host configuration is changed.

Seven cases run separately for each architecture: four Standard URL/bootstrap pairs (Cloudflare and Google over IPv4/IPv6), the two IPv4 pairs with StrictOffline, and invalid policy value 2. A minimal `example.com A IN` query records typed error, returned length, message ID/QR/RCODE and the complete wire question (case-insensitive QNAME plus exact QTYPE/QCLASS). IPv6 is recorded as `executed=0` when no usable nonlocal address exists; it is never counted as a pass. The strict negative expectation on this host is `revocation_unknown`; invalid policy must return `argument` before network activity.

Each case installs the existing architecture-matched fixture API trap before loading the product DLL. Its endpoint allowance is immutable, so one process is used per pair/policy. The trap blocks/counts 18 Host DNS/PAC/Windows chain APIs, UDP send APIs, unexpected TCP endpoints and unsupported socket extensions. Standard and StrictOffline must make zero forbidden calls. This instrumentation does not cover Cryptnet URL cache APIs, direct AFD/system calls, other processes or global packets; cache-only flags remain a separately verified source contract.

The trap must first be built from `tools/envbox-dns-doh-fixture/trap` into `target/doh-api-trap64` (CMake architecture `x64`) and `target/doh-api-trap32` (`Win32`), using Release configuration. See the [trap instructions](../envbox-dns-doh-fixture/trap/README.md).

Run from the repository root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/run.ps1
Get-Content target/doh-native-acceptance-results.json -Raw
```

Every invocation creates a fresh `run_id` and waits up to four minutes for
that WMI worker's uniquely named result. The stable result/log paths above are
copied only after the matching run has completed. A completed harness records
`completed=true` and `harness_completed=true`. `native_ipv4_pass`,
`policy_negative_pass` and `process_api_pass` are independent checks; the script
exits zero only when all three pass. The wider `gate=false` remains because
this is not global observation or an OS matrix. `wire_positive` requires all
four Standard pairs on both architectures and remains false when IPv6 is
unexecuted. A stale result cannot satisfy this run.

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

This is a positive/negative native policy probe with process-local API tripwires, not global packet capture. A Standard success demonstrates the configured literal endpoint, URL identity and production trust for this Windows installation. StrictOffline `revocation_unknown` is an expected policy negative here. Other typed failures retain their reasons. The result does not prove every Cryptnet/AFD path is absent and does not establish an OS-version matrix.

The separate CMake target `config-policy-probe` compiles the production Runtime
DNS configuration decoder directly. Run its Release executable in a fresh,
uninjected process after building that target for x64 or Win32. Its seven cases
check complete Standard/StrictOffline wire roundtrips, rejection of missing or
unknown TLS policy, rejection of VirtualView `strict=0` and `false`, and Host
mode compatibility. It returns nonzero on any failed assertion; it does not
contact DNS services or load a Runtime DLL. This configuration test is separate
from the public native transport acceptance results.
