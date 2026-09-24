Parent: .scratch/envbox-v02/spec.md

# 24: DNS routing — 解析入口 Hook 设计

**What to build:** DnsMode `VirtualView` 下 per-process 解析路由的设计与 API 优先级。

**Blocked by:** 23 Audit Mode — Probe/验收对比（用 Audit 观测解析入口）

**Status:** open

- [ ] CONTEXT 增加 **DNS routing**（区别于 DNS View）
- [ ] 优先 API：`DnsQuery_A/W/UTF8/EX`、`getaddrinfo`、`GetAddrInfoW/Ex`
- [ ] 选定实现策略（Profile servers 转发 / 自定义解析）；Fail Open
- [ ] 明确非目标：WFP / LSP / DoH / 端口 53 透明重定向

## Comments
