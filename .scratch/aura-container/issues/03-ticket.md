# 03: 受控 bootstrap 及应用入口门控

Stage: P0
Status: claimed
Blocked by: [02: 真实 Runtime 身份与授权握手](02-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 受支持目标只有在实际 Runtime 身份和必要能力确认后才执行应用入口；失败不能先运行应用再报告启动拒绝。

## 负责模块与契约

Launcher 与 Runtime loader/bootstrap 门控。证明 Detours 初始挂起加载时序，loader lock 内不等待管理命令；遵守 caller-requested suspended。

## 不包括

不依赖初次 Resume 前自然产生 ACK；不覆盖尚未证明的目标类型，不通过禁用应用安全机制解决门控。

## 验收标准

- [ ] 夹具入口写入可观测标记；确认完成前标记不存在，确认后才出现。
- [ ] 配置、必要 Hook、身份、超时和后台断连失败均阻止应用入口并清理本次进程。
- [ ] bootstrap 无 loader-lock 死等，截止时间到达可回收全部引导资源。
- [ ] caller-requested suspended 保持明确挂起契约；无法支持组合明确拒绝且入口不执行。
- [ ] x64/x86 门控均有实际证据；不成立的技术路径保留阻塞，不改成提前执行。

## 验证证据

独立入口标记、Runtime ACK 时间和线程状态证据；保存成功/失败/超时及 suspended 组合记录。

只有受支持范围的正向门控与失败清理均被实际证明，本票才可完成并解除下游。失败报告、候选方案或 Unverified 结果只是进展记录，不能替代完成证据。

## 关联验收

A06、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
