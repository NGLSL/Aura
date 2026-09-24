Parent: .scratch/envbox-v01/spec.md

# 08: DNS View

**What to build:** Profile 为 VirtualView 时，目标进程枚举 DNS 配置看到 Profile 服务器列表；Mode 为 Host 时保持真实配置。仅虚拟化“读到的 DNS 视图”，不做任何流量拦截。

**Blocked by:** 05 Minimal Profile — 4 个核心 API

**Status:** done

- [x] Hook `GetNetworkParams` / `GetAdaptersAddresses`，按 DnsMode 返回 Host 或 Profile servers
- [x] Probe DNS 段在 US Profile + VirtualView 下显示 Profile 地址（如 1.1.1.1 / 1.0.0.1）
- [x] 不实现 raw UDP/TCP 53、DoH、WFP、透明代理
- [x] Host 全局 DNS 配置不被修改

## Comments

- Ticket 08 delivered. `hooks_dns.cpp` virtualizes **read** of DNS config only. `DnsMode::Host` is a pure pass-through. `DnsMode::VirtualView` replaces `GetNetworkParams` DnsServerList/CurrentDnsServer and each adapter `FirstDnsServerAddress`. No UDP/TCP 53, DoH, WFP, or proxy code exists.
- Review fixes (hard/wrong): parse scoped to `[profiles.dns]`; server overflow Fail Open (no silent truncate); VirtualView never leaves Host list (empty Profile IPv4 list still replaces Host); `GetAdaptersAddresses` filters by `Family` (AF_INET / AF_INET6 / AF_UNSPEC); `CurrentDnsServer` cleared/set to Profile.
- Policy: `GetNetworkParams`/`IP_ADDR_STRING` is IPv4-text only; IPv6-only VirtualView shows an empty IPv4 list (still not Host). IPv6 servers appear on `GetAdaptersAddresses`.
- Evidence: `cargo test --workspace` 59 passed — `run_probe_dns_virtual_view_shows_profile_servers` (exact GetNetworkParams set `1.1.1.1`/`1.0.0.1`, adapters contain both), `run_probe_dns_host_mode_matches_host` (both APIs equal Host).
- Deferred: IPv6 text form on Probe adapter listing is expanded hex (display only). `CreateProcessAsUser*` family remains P2 (ticket 06).