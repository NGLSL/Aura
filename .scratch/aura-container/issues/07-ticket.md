# 07: 不可变 Run 快照与 Profile 变更处理

Stage: P1
Status: claimed
Blocked by: [06: 持久 Container 创建编辑与版本化保存](06-ticket.md)、[02: 真实 Runtime 身份与授权握手](02-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 每次 Run 使用完整版本化有效快照；之后编辑或删除 Profile 不改变已有实例。

## 负责模块与契约

Core/Storage、请求解析和 bootstrap DTO。RuntimeInstance 关联 Container 与配置身份；完整性校验失败拒绝启动。

## 不包括

不实现 Supervisor 启动事务，不热更新 Runtime，不额外复制长期可编辑 Profile。

## 验收标准

- [ ] A 的 Run 快照包含完整有效配置、schema 和摘要，可按身份独立验证。
- [ ] 编辑 Container 默认 Profile 或原 Profile 后旧快照不变，新 Run 使用新配置。
- [ ] Profile 删除或无效使新 Run 失败；已有实例不改为 Host 或别的 Profile。
- [ ] 配置缺字段、损坏、超限或摘要不符不能静默过滤后继续。
- [ ] x64/x86 bootstrap 序列化一致，凭报文改 Instance/Container 不可领取其他快照。

## 验证证据

A/B 和编辑/删除 Profile fixture、快照 roundtrip、认证与配置损坏负向记录。

## 关联验收

A02、A03、A10、F01、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
