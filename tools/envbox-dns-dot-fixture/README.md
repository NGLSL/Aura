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
