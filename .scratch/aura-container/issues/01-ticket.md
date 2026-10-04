# 01: 启动失败句柄和资源清理

Stage: P0
Status: claimed
Blocked by: None (can start immediately)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 启动失败时只清理本次创建的资源，返回原始失败原因，不留下未受控应用，也不关闭其他对象。

## 负责模块与契约

Launcher、Runtime 进程创建适配和 Job ownership。明确 Detours 与调用者各自拥有的句柄；保持既有成功启动行为。

## 不包括

不增加启动入口、不改变 Profile 或网络策略。

## 验收标准

- [ ] 分别触发进程创建、注入、Job 分配及恢复线程失败，均保留正确原始错误。
- [ ] Detours 已关闭的句柄不重复关闭；成功转交后的句柄只有一个 owner。
- [ ] 失败只终止本次新建进程；既存宿主及其他实例继续运行。
- [ ] 连续失败与成功交替运行不泄漏 Process/Thread/Job handle。
- [ ] 受控 Probe 的 x64/x86 正常启动仍通过。

## 验证证据

记录失败注入点、进程退出与句柄生命周期；用受控夹具和未注入宿主对照，不以单纯构建成功代替清理证据。

## 关联验收

A06、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
