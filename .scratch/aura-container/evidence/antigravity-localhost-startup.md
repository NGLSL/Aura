# Antigravity localhost startup regression

Date: 2026-10-06

## Observed failure

Antigravity 2.19.1 language_server.exe exited naturally with code 2 under the existing US Development Profile. The fatal message was "listen tcp: lookup localhost: no such host". Host execution and an injected Host DNS Profile initialized normally.

Windows nested a private newer DnsQueryEx request inside the native GetAddrInfoW(localhost) path; Aura rejected the unsupported request version.

## Fix and boundary

runtime/src/hooks_dns.cpp scopes a thread-local permission to the synchronous native localhost/localhost. resolver call. Private layouts inside that scope remain interpreted by Windows. Public v1 requests still use normal validation/Profile routing, including real domains nested inside this scope. Unrelated calls cannot create the scope. Version identity equality between CLI and Runtime remains required.

Dual-architecture contracts pass, including native private reentry, trailing-dot/case handling, scope restoration, outside-scope rejection, and public real-domain requests inside the scope receiving the Profile answer without native fallback.

## Actual verification

Matched target/debug 0.3.8 CLI and freshly configured/built Runtime; controller PID 30164 had zero Runtime modules. Only copied Profile/configuration and isolated server data were used.

The owned language-server PID 25768 established localhost HTTPS port 12785 and HTTP port 12786, initialized in 12.2874987 seconds, and remained alive for the 30-second observation. Only the test-owned server was stopped afterward. Audit contains dns-localhost-native-reentry, no rejected/unsupported DNS entries, and remote Profile DoH success.

Ignored raw evidence: target/antigravity-profile-compare/red-evidence/ and green-evidence/; target/localhost-contracts-public-v1.{log,json}.

This validates the language-server startup regression. Full IDE login/function testing and a real installed upgrade are not covered.
