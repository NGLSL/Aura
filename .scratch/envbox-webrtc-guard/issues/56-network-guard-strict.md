Parent: .scratch/envbox-webrtc-guard/spec.md

# 56: Network Guard — Session 级 UDP 约束（Phase 3）

**What to build:** `WebRtcPolicy::Strict` 的执行层：Session 内 direct UDP 不放行，且不影响 Session 外同程序实例。

**Blocked by:** 55

**Status:** resolved

- [x] 语义：Session 进程树 UDP 出站仅允许 Profile 授权出口；无 UDP-capable proxy 则 direct UDP deny（Runtime WinSock hooks_network）
- [x] 不按 STUN/TURN 端口封禁
- [x] Session 归属查 Session Registry / Process Tracker（PID），禁止仅 `ALE_APP_ID` 当作产品承诺 — Runtime Hook 天然 Process-scoped
- [x] 用户态 WFP 仅作开发验证，文档标明局限（分不清 Aura Chrome vs 用户 Chrome）
- [x] Strict 无法执行 → Startup Fail Policy（禁止静默降级 Balanced）
- [x] stop 后约束移除；Host 防火墙/全局配置不变 — Hook 随进程生命周期
- [x] browser-probe 矩阵：Strict 无 direct UDP candidate；IPv6 不绕出；Proxy down 不直连
- [x] HTTP/3 回退 TCP 为可接受行为并写入文档

## Comments

### 2026-09-26 hardening（评审三项）

- [x] connected UDP：`connect`/`WSAConnect`/`send`/`WSASend`/`WSASendMsg` + per-socket `SOCK_DGRAM` state（不再只拦 `sendto`/`WSASendTo`）；`WSASocketA/W`/`socket` 建表，表满拒绝 socket，`sendto(NULL)` 回落 per-socket peer
- [x] UDP/53 收紧为 allowlist（禁止任意 IP:53）：VirtualView 仅 `dns_servers`（空列表 / 仅 loopback 可完全关闭外部 UDP/53）；Host 模式跟 host resolver（要用关闭语义请改 VirtualView）
- [x] Strict attach 不全 → Startup Fail（`EnvBoxInstallNetworkHooks` 返回 0，DllMain 拒绝启动）
- [x] browser-probe：IPv6 XOR-MAPPED-ADDRESS 按 RFC 5389 magic||txid 完整解码；Chromium/Edge `--browser` 真页 `RTCPeerConnection` + Intl 验收（mDNS 归 host、ProxyOnly+ 断言非 relay、Intl 全字段 + `navigator.languages`）；`--browser` 按 policy 自动注入 `--force-webrtc-ip-handling-policy`
