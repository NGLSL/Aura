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
and awaited. Deadline/cancel covers trust preparation and network work; local
CryptoAPI/registry calls have checks before/after and per item, but cannot be
preempted inside the native call. Rust unwinding panics are isolated at the FFI
boundary and asynchronous query boundary; allocator aborts or invalid caller
pointers are not recoverable Rust panics.

Offline trust reads bounded CurrentUser/LocalMachine physical registry ROOT,
CA and Disallowed snapshots, plus readonly landed Disallowed CTL cache. No
CryptoAPI chain, certificate URL retrieval, logical-store provider, SSL_CERT
environment override or host trust mutation is used. ROOT anchors require
effective serverAuth EKU/time eligibility. CA material remains intermediate
candidates. Explicit denied certificates and supported CTL subjects restrict
roots and peer certificates. Unknown CTL structure/algorithm fails closed.

Revocation requires a nonempty CRL collection, full non-root chain coverage,
known status and unexpired CRLs. The underlying rustls builder defaults to
chain checking and unknown-status denial, but empty CRLs disable its revocation
checks; this crate rejects that case explicitly and enables CRL expiration
enforcement. See the [rustls verifier builder](https://docs.rs/rustls/0.23.45/rustls/client/struct.ServerCertVerifierBuilder.html).

This is a narrower local policy than Windows' complete native trust engine.
Logical/Enterprise/GroupPolicy/SmartCard providers, all Windows CTL semantics and
Cryptnet URL-cache coverage are not reproduced. Public resolvers can fail when
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
