# DoT transport fixture

This fixture compiles the production DoT implementation directly, with
`ENVBOX_DNS_TRANSPORT_TESTING` enabled only on its own executable. Its explicit
chain-engine argument uses an exclusive memory root store and a memory CRL. It
does not install certificates, modify host trust, mutate network policy, or
provide a product environment variable that bypasses validation.

Build both architectures using CMake/MSVC, then run `run.py` with their executable
paths. The script requires Python `cryptography`; use an isolated environment
and an explicit pip `--target` inside that environment, since machine pip
configuration can redirect even a virtual environment's installation target.
Generated private keys and certificates stay in a unique `target/dot-cert-*`
directory. The server binds only a dynamically selected loopback port and each
client has a bounded timeout. Server threads close their own sockets.

```powershell
cmake -S tools/envbox-dns-dot-fixture -B target/dot-fixture64 -A x64
cmake --build target/dot-fixture64 --config Release
cmake -S tools/envbox-dns-dot-fixture -B target/dot-fixture32 -A Win32
cmake --build target/dot-fixture32 --config Release
python tools/envbox-dns-dot-fixture/run.py `
  target/dot-fixture64/Release/envbox-dns-dot-fixture.exe `
  target/dot-fixture32/Release/envbox-dns-dot-fixture.exe
```

For a public production-trust smoke, use `run_public_injected.py` with the
real CLI, Probe and one architecture's Runtime DLL:

```powershell
python tools/envbox-dns-dot-fixture/run_public_injected.py `
  target\debug\envbox.exe target\debug\envbox-probe.exe `
  target\nonvm-final-runtime-v2\envbox-runtime64.dll
```

The default endpoint is the literal IPv4 address `1.1.1.1:853` with TLS
identity `cloudflare-dns.com`; no IPv6 endpoint or Host fallback is added.
Each invocation is exactly one attempt and writes a unique
`target/dot-public-injected-evidence-<uuid>.json` file. That evidence records
the cleared `ENVBOX_*` keys, CLI/Probe/DLL paths and SHA-256 values, a fresh
uninjected Host Probe, raw Probe output, and the live Probe Runtime module
path/hash. A Runtime module already loaded in the Python controller aborts the
run, and temporary configuration is deleted only after a resolved `target`
containment check.

The final 2026-10-06 single attempts used the selected `target/nonvm-final-runtime-v2`
pair, with the x64 CLI driving both architecture probes: x64 CLI + x64 Probe +
`envbox-runtime64.dll`, then x64 CLI + x86 Probe-only + `envbox-runtime32.dll`.
Both attempts completed A/W/UTF8/Ex/async with `DnsRR_Status=0` and
`DnsRR_Records=1`:

- x64: `target/dot-public-injected-v2-x64-single.log`, evidence
  `target/dot-public-injected-evidence-2b2a6cf2567b47b6b874547c9b286b7c.json`;
- x86: `target/dot-public-injected-v2-x86-single.log`, evidence
  `target/dot-public-injected-evidence-746ea8ca4ba14aa785591039983aa6dc.json`.

The selected Runtime SHA-256 values are x64
`CA8283ADAE000DBEAAE65902A10F2E0E05B94C38C14F7B3652EDD3066C43D965` and x86
`5D2FD1038748F0D579F5B2EB59EBAEECDFF6C7846D12FD93876875696B6CEBE7`.
Each run was `attempt=1`, `retry=false`; the older V3 timeout/retry logs remain
historical samples and are not mixed into this pair's result. Any later async
Runtime freeze must be tested once per architecture with separate evidence.

The fixture covers DNS name and IP SAN success, fragmented TLS/framing, maximum
65535-byte packets, 16 sequential connections with handle counts after warmup,
untrusted/expired/mismatched/revoked certificates, absent local CRL, truncated
frames, two DNS responses in one TLS record, handshake/read timeout and cancel.
Certificate failures must occur before the server receives any DNS application
data. These are direct transport tests, not injected Profile/CLI tests.

The first implementation deliberately negotiates TLS 1.2 only using the
deprecated but supported `SCHANNEL_CRED` interface. There is no connection pool;
each query closes its socket and Schannel context. Production verification uses
Windows local trust and requires cached revocation evidence: offline/unknown
revocation is an error, distinct from revocation. Some public DoT services will
therefore fail when the required material is absent. Only the Query Engine can
select another explicitly configured upstream.

Certificate-chain construction is synchronous and restricted to local material.
Deadline/cancel is checked before and after it; local CryptoAPI execution cannot
be interrupted midway. Network waits poll in slices of at most 50 ms, subject to
Windows scheduling. The fixture does not constitute a full network capture of
all Windows auxiliary traffic, or evidence of product injection/DoH support.
