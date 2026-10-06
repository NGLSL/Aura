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

同日后续：已补 native Cryptnet CRL cache-only/no-write 读取和严格标准 verifier 重试，自包含 fixture 不读宿主缓存。用户 cache 的约 80 条列表不代表目标链可用；双架构 Google peer CDP 实际命中 2 份候选，Cloudflare 命中 0，研究 AuthRoot 变体仍严格拒绝 unknown revocation。AuthRoot CTL 验签、策略语义及 signer 轮换研究已保存，生产路径未全量导入 AuthRoot。见 [缓存切片证据](../evidence/doh-offline-crl-cache.md)、[AuthRoot 离线研究](../evidence/authroot-offline-research.md)。18 No-Go/19 阻塞保持。

前一切片新增的 native cache 步骤间取消门禁已在后续切片补齐：caller-thread RAII scope 保管完整 Budget，verifier 只保留不可复用的 thread/scope identity；缓存 collector 各 CAPI 步骤间检查原 callback，一次性取消被锁存并传回 transport。双架构真实 cache-only collector 的入口、CDP 返回后和 retrieval 返回后取消均通过，查询返回后 callback 已恢复零，HTTP/canary 仍为零。单次同步 CAPI 内部不能抢占。另已实现受控 signed CTL 的显式 signer pins、逐 signer 验签和验签后策略检查，但不接入生产 AuthRoot；真实 policy/provenance/rotation 等门槛仍未解除。见 [本切片证据](../evidence/doh-signed-ctl-cancellation.md) 与 [独立审查](../evidence/doh-signed-ctl-cancellation-review.md)。18 No-Go/19 阻塞保持。

## 2026-10-06 用户授权后的标准 TLS 修复

用户明确要求修复，不再将 DNS strict 与完整离线 revocation 绑定为默认门槛。本票历史 No-Go 对应旧策略，保留为历史证据；当前 Standard / StrictOffline 是显式分开的策略。加入固定 Mozilla DER 根并修正 Google HTTP/2 对额外 Host 的兼容问题后，当前 Windows x64/x86 默认 native C ABI 的 Cloudflare/Google IPv4 均成功，policy negative 与 process API 分项均通过。114 fixture 场景通过；最终实际 Profile/Runtime 注入 16 项关键场景通过，另保留先前 24 项广覆盖。

当前机器的 Standard IPv4 后端资格已有真实正负证据，用户授权修复范围内已接入 19；不把固定根包当作全量 AuthRoot 授权。其他 OS/IPv6/global traffic 完整门禁仍 partial，宽泛 global acceptance 仍未通过，不能把本票或完整 Container 全部关闭。策略变更、来源/hash、阶段差异和剩余范围见 [本次修复证据](../evidence/doh-standard-tls-runtime.md)、[独立审查](../evidence/doh-standard-tls-runtime-review.md)。未修改宿主配置或加载驱动。
