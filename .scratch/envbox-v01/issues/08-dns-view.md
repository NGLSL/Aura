Parent: .scratch/envbox-v01/spec.md

# 08: DNS View

**What to build:** Profile 为 VirtualView 时，目标进程枚举 DNS 配置看到 Profile 服务器列表；Mode 为 Host 时保持真实配置。仅虚拟化“读到的 DNS 视图”，不做任何流量拦截。

**Blocked by:** 05 Minimal Profile — 4 个核心 API

**Status:** ready-for-agent

- [ ] Hook `GetNetworkParams` / `GetAdaptersAddresses`，按 DnsMode 返回 Host 或 Profile servers
- [ ] Probe DNS 段在 US Profile + VirtualView 下显示 Profile 地址（如 1.1.1.1 / 1.0.0.1）
- [ ] 不实现 raw UDP/TCP 53、DoH、WFP、透明代理
- [ ] Host 全局 DNS 配置不被修改
