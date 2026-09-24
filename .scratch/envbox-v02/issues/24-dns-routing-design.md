Parent: .scratch/envbox-v02/spec.md

# 24: DNS routing — 解析入口 Hook 设计

**What to build:** DnsMode `VirtualView` 下 per-process 解析路由的设计与 API 优先级。

**Blocked by:** 23 Audit Mode — Probe/验收对比（用 Audit 观测解析入口）

**Status:** resolved

- [x] CONTEXT 增加 **DNS routing**（区别于 DNS View）
- [x] 优先 API：`DnsQuery_A/W/UTF8/EX`、`getaddrinfo`、`GetAddrInfoW/Ex`
- [x] 选定实现策略（Profile servers 转发 / 自定义解析）；Fail Open
- [x] 明确非目标：WFP / LSP / DoH / 端口 53 透明重定向

## Comments

### 2026-09-24 design

**Decision: 自定义最小 DNS 客户端（UDP/53）**

- 在 `hooks_dns.cpp` 内实现 wire 协议客户端：按 Profile `dns_servers` 顺序对 A(1)/AAAA(28) 发 UDP 查询；单查询超时 1800ms，总预算 6000ms（永不永久挂起）。
- **有效应答（含 NXDOMAIN / NOERROR 空应答）是最终结果**，不得 Fail Open 漏到 Host DNS。
- 全部 server 不可达/超时/非权威失败（SERVFAIL/REFUSED）→ Fail Open 调原 API。
- 数字 IP / `AI_NUMERICHOST` / 仅 service 名 / 非 ASCII 节点 → 直通原 API，不改写。
- Host 模式（`DnsMode=Host` 或无 servers）**不安装**解析 hook（解析路径与 V0.1 完全一致）。

**优先 API 与实现范围**

| API | 策略 |
|-----|------|
| `getaddrinfo` / `GetAddrInfoW` | 必须真路由；完整 `addrinfo` 链（TCP+UDP 展开） |
| `GetAddrInfoExA/W` | 同步路径真路由；`lpOverlapped`/`lpCompletionRoutine` 异步 → Fail Open |
| `DnsQuery_A/W/UTF8` | A/AAAA 的 `DNS_RECORD`；其它 type → Fail Open |
| `DnsQueryEx` | 异步完成形态，不 hook（Fail Open by design），audit 记 `fail-open-unhooked` |
| `freeaddrinfo` / `FreeAddrInfoW/ExA/ExW` / `DnsFree` | 钩住以正确释放自建节点（owned registry） |

**非目标（明确不做）**

- WFP Driver / LSP / DoH 拦截 / 端口 53 透明重定向 / 系统代理
- 修改 Host DNS 配置或其它进程解析路径
- 完整 DNSSEC / EDNS / TCP fallback（超大应答截断时 Fail Open）

**CONTEXT:** `docs/CONTEXT.md` 已在 ticket 20 增加 **DNS routing** 术语（区别于 DNS View），本设计不重复大改 CONTEXT。
