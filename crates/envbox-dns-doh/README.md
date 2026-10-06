# Explicit-bootstrap DoH backend

This crate is linked into the x64/x86 C++ Runtime as a staticlib. Profile DoH
uses the existing Query Engine and DNS packet parser; TLS and HTTP transport
do not interpret QTYPE. Runtime capabilities advertise the linked backend.

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
Native CRL cache lookup recovers the shared budget from a caller-thread RAII
scope and checks cancellation between CAPI calls. The Send + Sync verifier
stores only a thread/scope identity; retired or foreign-thread identities fail
closed without invoking callbacks. No registry borrow spans a callback.
Cancellation is latched so a one-shot notification survives TLS error mapping
and cleanup. Transport also checks before HTTP and after cleanup. A single
synchronous native call cannot be preempted mid-call.
Rust unwinding panics are isolated at the FFI
boundary and asynchronous query boundary; allocator aborts or invalid caller
pointers are not recoverable Rust panics.

Offline trust reads bounded CurrentUser/LocalMachine physical registry ROOT,
CA and Disallowed snapshots, plus readonly landed Disallowed CTL cache. No
CryptoAPI chain, online certificate retrieval, logical-store provider, SSL_CERT
environment override or host trust mutation is used. A fixed-version
`webpki-root-certs` Mozilla public DER bundle supplies additional explicit
application anchors, filtered by the same local deny/signature-hash policy.
It is updated with dependencies/application releases, never during a query.
ROOT anchors require
effective serverAuth EKU/time eligibility. CA material remains intermediate
candidates. Explicit denied certificates and supported CTL subjects restrict
roots and peer certificates. Unknown CTL structure/algorithm fails closed.

TLS revocation is independent of DNS strict/no-host-fallback. Profile DoH and
the original `envbox_doh_query` entry use Standard by default: normal chain,
hostname, purpose, time and signature validation, plus known-revoked rejection
from available CRLs, without requiring complete revocation coverage or CRL
freshness. Standard does not enter the CDP cache collector. It never performs
online certificate/revocation retrieval.

`StrictOffline` requires a nonempty CRL collection, full non-root chain coverage,
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
absent from product builds and the C ABI. Fixture-only phase instrumentation
also exercises the real cache-only collector with a one-shot cancellation at
entry, after CDP enumeration and after retrieval. The public default trust
acceptance gate remains separate.

The test/fixture-only `signed_ctl` module verifies controlled signed CTLs with
explicit DER signer pins in a memory store. Every signer is verified before
policy metadata is read; list identity, time, sequence rollback and equal-sequence
digest checks fail closed. Unknown policy attributes are rejected. This module
does not authorize production AuthRoot anchors; signer provenance, rotation,
chain/revocation and real Root Program policy semantics remain unproved. See
[controlled fixtures](../../tools/envbox-authroot-fixture/README.md).

This is an application trust policy, with narrower Windows provider/policy
coverage than the complete native trust engine.
Logical/Enterprise/GroupPolicy/SmartCard providers, all Windows CTL semantics and
complete Cryptnet/OCSP/delta-CRL coverage are not reproduced. Public resolvers can fail when
matching offline revocation material is unavailable; no Host fallback follows.

`include/envbox_dns_doh.h` specifies the C ABI and typed error values. The
`envbox_doh_query_with_policy` entry accepts 0 Standard or 1 StrictOffline;
unknown values fail as Argument before trust/network work. Buffer and
callback lifetimes are caller obligations. The safe Rust deadline constructor
has no callback; the callback constructor is explicitly unsafe and documents
validity for every use and copy of the budget.

Product staticlib builds must use `--no-default-features --locked` and a target
directory separate from fixture builds. `fixture-trust` only exposes the
standalone Rust fixture API and must never enter a product staticlib. The fixture
binary is gated by `required-features`, so ordinary workspace builds do not
activate it. The MSVC fixture verifies both architecture libraries really link
and contain no fixture API symbols. Runtime CMake builds the locked default
staticlib in its own architecture/configuration target directory and links the
reported native libraries; no fixture feature enters the shipped Runtime.
