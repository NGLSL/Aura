# Explicit-bootstrap DoH backend prototype

This crate is a prototype and FFI substrate. It is not enabled by the product
Query Engine or Runtime capability report. Product integration requires a
separate acceptance decision; the source does not replace the DNS packet parser.

The caller supplies a HTTPS URL, one literal bootstrap IP, a DNS packet, an
absolute Windows `GetTickCount64` deadline, cancellation callback and output
buffer. The URL alone determines the port. Ordered bootstrap/upstream retries
belong to the caller and must reuse the same absolute deadline.

Tokio connects a concrete `SocketAddr`; tokio-rustls uses ring with TLS 1.2/1.3.
Hyper's low-level `client::conn` consumes that already-connected TLS stream and
selects HTTP/2 or HTTP/1.1 by ALPN. There is no high-level HTTP client, hostname
resolver, proxy/PAC lookup, redirect support, automatic decompression or pool.
Each query owns one connection; pool capacity and idle lifetime are both zero.

HTTP policy uses POST with `application/dns-message`; only 2xx responses pass.
Content-Type is parsed by the `mime` library, allowing valid case and parameters.
Content-Encoding must be absent or one `identity` value. Header/trailer budgets
are 64 fields and 32 KiB, and the complete response must contain 12..65535 bytes.
Both Content-Length and streamed bodies enforce the byte limit.

The current-thread runtime uses a bounded tracked executor for Hyper connection
and HTTP/2 tasks. At exit, new work is disabled and every owned task is aborted
and awaited. Trust preparation and network work check deadline/cancellation.
Native CRL cache lookup within the TLS verifier receives a deadline-only budget;
it cannot promptly observe caller cancellation between CAPI calls. Transport
checks cancellation after TLS before HTTP and again after cleanup. Local native
calls cannot be preempted mid-call. This limitation remains an enablement gate.
Rust unwinding panics are isolated at the FFI
boundary and asynchronous query boundary; allocator aborts or invalid caller
pointers are not recoverable Rust panics.

Offline trust reads bounded CurrentUser/LocalMachine physical registry ROOT,
CA and Disallowed snapshots, plus readonly landed Disallowed CTL cache. No
CryptoAPI chain, online certificate retrieval, logical-store provider, SSL_CERT
environment override or host trust mutation is used. ROOT anchors require
effective serverAuth EKU/time eligibility. CA material remains intermediate
candidates. Explicit denied certificates and supported CTL subjects restrict
roots and peer certificates. Unknown CTL structure/algorithm fails closed.

Revocation requires a nonempty CRL collection, full non-root chain coverage,
known status and unexpired CRLs. The underlying rustls builder defaults to
chain checking and unknown-status denial, but empty CRLs disable its revocation
checks. Fixture snapshots reject empty lists during preparation. Native
verification may read existing HTTP(S) CDP cache entries with fixed
`CRYPT_CACHE_ONLY_RETRIEVAL | CRYPT_DONT_CACHE_RESULT` flags, but never accepts
an empty-list verification result. Only unknown/expired revocation triggers
this lookup; all candidates go through the same strict standard verifier.
Cache misses, malformed entries and incomplete chains remain failures. The
reader bounds certificates, URLs, bytes and deadline; it never downloads or
writes cache/store material. CRL expiration enforcement remains enabled. See
the [rustls verifier builder](https://docs.rs/rustls/0.23.45/rustls/client/struct.ServerCertVerifierBuilder.html).

The standalone `fixture-trust` feature can supply explicit cache candidate DER
to exercise that same retry branch without touching host caches. This input is
absent from product builds and the C ABI. Native cache work currently has only
a deadline budget; prompt cancellation between CAPI calls remains a product
enablement gate. Transport checks cancellation after TLS before HTTP.

This is a narrower local policy than Windows' complete native trust engine.
Logical/Enterprise/GroupPolicy/SmartCard providers, all Windows CTL semantics and
complete Cryptnet/OCSP/delta-CRL coverage are not reproduced. Public resolvers can fail when
matching offline revocation material is unavailable; no Host fallback follows.

`include/envbox_dns_doh.h` specifies the C ABI and typed error values. Buffer and
callback lifetimes are caller obligations. The safe Rust deadline constructor
has no callback; the callback constructor is explicitly unsafe and documents
validity for every use and copy of the budget.

Product staticlib builds must use `--no-default-features --locked` and a target
directory separate from fixture builds. `fixture-trust` only exposes the
standalone Rust fixture API and must never enter a product staticlib. The fixture
binary is gated by `required-features`, so ordinary workspace builds do not
activate it. The MSVC fixture verifies both architecture libraries really link
and contain no fixture API symbols. No product CMake integration is enabled.
