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
