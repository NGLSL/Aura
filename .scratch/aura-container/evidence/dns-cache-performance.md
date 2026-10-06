# DNS response cache performance regression

Date: 2026-10-07

## Reproduction

`tools/envbox-dns-bootstrap-fixture/contracts.cpp` exercises the real `RouteDnsQuery` and `DnsQueryOne` code with a controlled transport seam and native Windows record conversion/freeing. Before caching, 100 repeated A, HTTPS and SVCB queries each reached the DoH transport 100 times. This is the repeated-work symptom; mock transport timings alone do not establish public DNS latency.

## Change and boundaries

The new process-local wire cache sits above UDP/TCP/DoT/DoH. Its key includes Profile identity, configured endpoint and TLS policy, normalized question, QTYPE and options. It caches positive, validated responses, with expiry bounded by the shortest ordinary RR TTL and a 60-second ceiling. TTL zero, malformed data, truncated packets, negative answers and transport/decode failures are not admitted. Returned TTLs age; OPT metadata remains unchanged.

The table has at most 64 entries and a memory budget below 1 MiB, counting wire bytes, key capacities and TTL metadata. Cached packets are copied before use and receive the new request transaction ID. Each caller receives separately allocated native records, released by the original Windows free API.

A 32-slot in-flight table merges concurrent misses for the same key. Network calls happen outside its lock. Waiters retain their cancellation signal and original absolute deadline. Explicit BYPASS_CACHE, WIRE_ONLY and DONT_RESET_TTL_VALUES requests bypass the new cache. Bootstrap A/CNAME responses use this same cache and continue to come only from connectable Profile upstreams.

This does not add Host fallback, disable certificate validation, reuse permanent certificate trust snapshots or implement persistent TLS connection pooling. The old address-only resolver cache remains separate.

## Native verification

Both x64 and x86 Release fixtures passed:

| Case | Before | After |
| --- | --- | --- |
| 100 repeated A queries | 100 DoH transport calls | 1 |
| 100 repeated HTTPS queries | 100 DoH transport calls | 1 |
| 100 repeated SVCB queries | 100 DoH transport calls | 1 |
| Eight simultaneous cold queries for one key | No merging | 1 transport call |
| Two business names using hostname DoH bootstrap | Repeated seed resolution | 2 total seed calls for the CNAME chain |

For a deliberately simulated 10ms transport delay, x64 measured 20 bypass queries at 305.87ms and 20 cache-enabled queries at 16.21ms; x86 measured 321.20ms and 16.24ms. Windows Sleep granularity affects these figures. They are controlled-fixture results, not measured public DoH speedups.

Waiter cancellation returned in 31ms on both architectures; deadline expiration returned in 16ms/x64 and 15ms/x86 without another transport request. The standalone cache fixture also covers TTL ageing/expiry/zero, CNAME compression, malformed wire data, OPT, key separation, bounded eviction and concurrent access.

Raw logs are under ignored `target/dns-cache-route-green64.log` and `target/dns-cache-route-green32.log`. CMake fixtures live in `tools/envbox-dns-cache-fixture` and `tools/envbox-dns-bootstrap-fixture`; CI runs both for x64 and Win32.

## Injected acceptance

The Probe supports bounded `--dns-rr NAME QTYPE API OPTIONS --repeat N`. `full_qtype_response_cache_reuses_profile_wire_and_honors_bypass` runs actual injected A/W/UTF8/Ex/async queries for A, SVCB, HTTPS, TXT and a private unknown type. Each process makes 20 queries, checks all returned records are present and freed, and counts the local fixture's real UDP packets: one with caching, twenty with explicit bypass.

`cargo test -p envbox-cli --test cli_dns --locked -- --test-threads=1` passed all 34 tests with the freshly built x64 Runtime. This includes the new repeated-query acceptance and the existing transport, no-Host-fallback, native-localhost, cancellation and malformed-response checks. Both Runtime DLL architectures also built successfully.

The existing launch-latency acceptance passed separately: best virtualized launch 88.80ms, plain launch 9.57ms, extra launch cost 79.23ms. This is startup evidence, not a DNS network-latency benchmark. Independent cache risk review found no actionable new blocker; the older address-only cache TTL policy and lack of persistent TLS pooling remain explicit boundaries.

Public-network latency, extended browser/IDE workloads and real installation/upgrade validation remain separate from these cache regression checks.
