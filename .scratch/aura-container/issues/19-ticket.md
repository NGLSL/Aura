# 19: DoH 正式 transport 实现

Stage: P3
Status: claimed
Blocked by: [18: DoH 无宿主 DNS bootstrap 选型原型](18-ticket.md)、[15: 统一任意 QTYPE Query Engine 与 UDP/TCP](15-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 通过已证明的 HTTP/TLS backend 将 DNS wire POST 发到配置 DoH 上游，严格保持 bootstrap 与 TLS 契约。

## 负责模块与契约

Runtime DoH transport、能力报告和连接生命周期。仅在 18 有通过门禁后接入 Query Engine，Profile 数据面不依赖 GUI。

## 不包括

不允许 Host bootstrap/PAC/继承代理/凭据、不自动跟随重定向、不增加通用 HTTP 客户端功能。

## 验收标准

- [ ] HTTPS POST 使用 application/dns-message 的 Content-Type/Accept，任意 QTYPE 使用统一响应校验。
- [ ] 连接到显式 bootstrap，同时校验 URL 主机身份；全部 bootstrap 失败不切 Host。
- [ ] 非 2xx、错误媒体类型、超大/畸形 body、重定向均明确失败并按配置顺序处理。
- [ ] 请求、连接和 TLS 共享 deadline；取消不产生重复回调或后续上游尝试，池容量与空闲回收可验证。
- [ ] x64/x86 打包及实际注入符合原型身份/零 Host DNS 契约；缺失 backend 能力拒绝请求。

## 验证证据

本地 HTTP/TLS wire fixture和负向集、bootstrap抓包与原型对照；实际远端服务仅在允许的只读测试中核验。

## 关联验收

F03。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：前置资格未通过，未执行正式后端。19 等待 DoH bootstrap 隔离 Go；21 已完成只读预检，但无 WDK、隔离 VM 和相应测试材料。用户确认尚无 VM，先完成独立部分；不在宿主安装驱动。见 [实施进度](../evidence/implementation-progress.md)。

2026-10-06 用户明确授权修复：拆开 DNS strict 与 TLS revocation，固定 Mozilla 公共根和标准证书验证成为缺省，StrictOffline 显式保留。当前 Windows x64/x86 公共 native IPv4 正反向/process API 分项资格通过后，Runtime 已必链接 Rust staticlib并接入 Query Engine，能力报告 dns_doh=1；GUI/CLI/IPC/immutable snapshot完整传递策略。最终实际 Profile 注入 16 项关键验收通过，涵盖Cloudflare/Google A/HTTPS65、严格失败、bootstrap重试/all-dead和受控取消；HTTP2/HTTP1、标准/严格fixture114通过。

本票当前为 implemented / partial acceptance；缺少安装升级、其他目标OS/IPv6/global traffic完整证据，未认领整票或P0–P8全部完成。Blocked-by保留用于完整最终验收，当前机器可用性资格与更广Container保证分项记录。见 [本次修复证据](../evidence/doh-standard-tls-runtime.md)、[review](../evidence/doh-standard-tls-runtime-review.md)。
