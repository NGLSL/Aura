# 18: DoH 无宿主 DNS bootstrap 选型原型

Stage: P3
Status: claimed
Blocked by: [14: 有序 typed DNS 配置全链及迁移](14-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 验证 DoH 能以指定 bootstrap IP 连接，同时保留 URL authority、SNI、证书身份和 HTTP/2，确定可进入正式实现的技术方案。

## 负责模块与契约

隔离原型、Windows HTTP/TLS 能力与选型证据。优先验证 WinHTTP；目标系统、x64/x86 和辅助证书流量均单独记录。

## 不包括

不交付正式 DoH、不手写完整 HTTP/2、不配置宿主代理、不自动申请外部证书或部署服务。

## 验收标准

- [ ] 主机名 URL 的连接实际远端为配置 bootstrap IP，Host/SNI 和校验证书身份仍为 URL 名称。
- [ ] 正向成功与错误身份/证书、过期证书均按契约处理；证书校验不关闭。
- [ ] 观测到 HTTP/2 能力且无 Host DNS/PAC/隐式代理/重定向解析，辅助证书请求也有记录。
- [ ] 目标系统和 x64/x86 结果分别可复现；成功不从文档或单平台结果推断。
- [ ] 结论明确 Go/No-Go；不达标保留本票门禁，评估成熟库成本与方案后重新证明，不默默放行正式实现。

## 验证证据

原型材料、夹具日志、实际远端 IP、Host/SNI/协议和抓包；附失败方案与可核查选型结论。

只有选定方案满足上述正向门槛且负向验证通过，本票才可完成并解除 19 的阻塞。No-Go 报告和替代库评估属于进展记录，未验证替代方案不能视作门禁通过。

## 关联验收

F03、F10。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。

后续增量：Rustls/Hyper 显式 socket 候选已完成本机双架构原型，56 个 HTTP/TLS 场景及原生 C++ FFI 验证通过；本地 native cached Disallowed/signature-hash 与严格离线 CRL 已实现。原生系统信任公共目标、IPv6、其他 OS 与完整系统观测尚未证，整体 No-Go 保留；见 [原型证据](../evidence/doh-rustls-prototype.md) 与 [研究设计](../evidence/doh-rustls-design.md)。本轮不解除 19 的门禁。

2026-10-06 增量：本地双架构矩阵新增 IPv4 URL/IP SAN 情况，实际执行 **58 个场景通过**；用户确认宿主禁用 IPv6，10 个 IPv6 场景记录为未执行，不计入通过数，未修改宿主配置。真实 product-default staticlib → MSVC DLL → C ABI 的公共 IPv4 验收仍返回 certificate failure；独立原生诊断定位 `UnknownIssuer`，研究用途加入本机 AuthRoot 候选后又得到 `UnknownRevocationStatus`。AuthRoot CTL 的严格安全接入与足够的离线吊销材料尚未证明；不能全量信任 AuthRoot、关闭校验或隐式下载材料来转绿。这些是信任实现/材料门槛，与 VM 无关。整体 No-Go 和 19 门禁保留。见 [IPv4/IPv6 分项证据](../evidence/doh-ipv6-independent.md) 与 [原生信任诊断](../evidence/doh-native-acceptance.md)。
