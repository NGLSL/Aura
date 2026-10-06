# Signed CTL research fixtures

These are synthetic, public fixtures for `envbox-dns-doh::signed_ctl`, not
Microsoft AuthRoot data or production trust anchors. The research module is
compiled only for tests or the standalone `fixture-trust` feature.

`generate.py` uses an existing Python with `cryptography` installed. Keys are
generated and used entirely in the Python process. It writes only public signer
DER certificates and signed CTL messages under `fixtures/`; it does not install
certificates, create Windows key containers, use system stores, or access the
network. Regeneration deliberately produces new keys and signatures. Commit
the generated fixtures together so signer DER and signed messages match.

The fixed evaluation time is 2026-10-06 00:00 UTC. Normal signer certificates
are valid from 2025 through 2035. The normal CTL is valid from 2026-10-01 until
2026-11-01, list identifier `Aura research v1`, sequence 7, and contains one
SHA-256 subject identifier with no authorization attributes. A SHA-1 subject
identifier is supported for compatibility, but SHA-1 message signatures are
rejected. Unknown entry attributes, extensions, signer attributes and algorithm
parameters are unsupported and rejected.

Run the actual native CryptoAPI tests on each architecture:

```powershell
cargo +1.99.0 test --locked -p envbox-dns-doh signed_ctl -- --nocapture
cargo +1.99.0 test --locked -p envbox-dns-doh --target i686-pc-windows-msvc signed_ctl -- --nocapture
```

The verifier creates a memory-only store containing exactly the explicitly
provided DER pins. It validates every message signer with both
`CMSG_TRUSTED_SIGNER_FLAG` and `CMSG_USE_SIGNER_INDEX_FLAG`, then checks the DER
pin, RSA key size, SHA-2 algorithms, Root List Signer EKU and evaluation-time
validity. Metadata is interpreted only after all signers authenticate. Rollback
comparison uses bounded unsigned little-endian sequence integers for the exact
expected list identifier. Equal sequence requires the previous encoded message
SHA-256 to match; no persistent rollback state is written.

The pins and evaluation clock are explicit inputs, not proof of trusted
provenance. Signer chain trust, revocation, publisher rotation, actual AuthRoot
policy attributes and root materialization remain separate production gates.
Synchronous CryptoAPI operations cannot be interrupted inside a single call;
the shared budget is checked between bounded operations.

Primary API contracts:

- [CryptMsgGetAndVerifySigner](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-cryptmsggetandverifysigner)
- [CryptMsgSignCTL](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-cryptmsgsignctl)
- [CertCreateCTLContext](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certcreatectlcontext)
- [CertOpenStore](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certopenstore)

The fixture PKCS#7 signs the content value octets according to RFC 2315 section
9.3; actual Windows verification in the tests establishes interoperability.
