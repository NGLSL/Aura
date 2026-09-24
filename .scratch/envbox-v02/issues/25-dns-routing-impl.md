Parent: .scratch/envbox-v02/spec.md

# 25: DNS routing — 实现 + Fail Open

**What to build:** 在 `hooks_dns.cpp` 实现 VirtualView 解析路由。

**Blocked by:** 24 DNS routing — 解析入口 Hook 设计

**Status:** resolved

- [x] VirtualView：按 Profile `servers` 顺序解析
- [x] 全部失败 → Fail Open 到原 API
- [x] Host 模式：解析路径完全不拦截
- [x] 仅 Process Tree Instance 生效
- [x] 单测/集成不依赖公网唯一路径

## Comments

### 2026-09-24 implementation

- 仅 VirtualView + servers 安装解析 hook；Host 路径与 V0.1 一致
- getaddrinfo / GetAddrInfoW / GetAddrInfoExA/W（sync）完整 addrinfo 链；DnsQuery_A/W/UTF8 返回 A/AAAA DNS_RECORD
- 单查询超时 1800ms、总预算 6000ms；全部失败 Fail Open 到原 API
- 数字 IP / AI_NUMERICHOST 直通；audit 记 virtualized + 节点摘要

### 2026-09-24 review fixes (unified)

- TC 截断 → Fail Open；CNAME 链（≤8）无 A/AAAA 不伪造 NXDOMAIN
- DnsQuery_UTF8 Fail Open 走 TrueDnsQuery_UTF8；非 ASCII 节点直通
- 自建节点 owned free；Owned 表满 HeapFree 并 Fail Open
- HookDnsQuery_W 入口保存 GetLastError
- 测试接缝 `ENVBOX_DNS_UDP_PORT`（本机 53 被 DNS 代理占用时用高位端口 fixture）
