# 12: Supervisor 崩溃恢复与 TrackingLost

Stage: P2
Status: claimed
Blocked by: [11: GUI 重连与幂等 Stop/Stop all](11-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 后台重启仅恢复可证明身份与控制的实例，其余显示 TrackingLost；A 失联不阻断独立 B。

## 负责模块与契约

Supervisor 持久记录、Job 重开和恢复状态机。校验 PID generation、Runtime、快照和有效控制句柄，不能凭旧 PID 恢复。

## 不包括

不自动重启应用、不强制杀失联实例、不把一次失联扩成全局停用。

## 验收标准

- [ ] 后台崩溃不自动终止所有目标；重启后可证明控制和身份的实例恢复 Running。
- [ ] 原实例仍存活但 Job/Runtime/快照或控制无法确认时进入 TrackingLost；确认旧实例及其 Job 成员不存在则结束旧记录。PID 已被新 generation 的宿主进程复用时不认领、不 Stop，也不把该宿主进程当成原实例等待退出或阻断新 Run。
- [ ] A 的 TrackingLost 阻止 A 新 Run；身份和策略完整的 B 可继续启动和停止。
- [ ] A 恢复控制必须重验全部身份与策略；仅改状态或再 LoadLibrary 不算恢复。
- [ ] OS 重启确认旧进程不存在后结束旧记录，不认领复用 PID，不自动拉起目标。

## 验证证据

故障终止后台、Job/身份损坏、PID generation fixture与 OS 重启验证分别归档；未执行系统重启不可标该项通过。

## 关联验收

A05、A15、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。

2026-10-06 增量：恢复失败的 `TrackingLost` 状态与原因现在写回持久记录。NoJob 下即使逐一确认最后 sealed PID generations 已不存在，也仍保持 `TrackingLost`，因为最后成员列表不能证明完整进程树没有未知后代；真实 Supervisor/不存在 PID fixture 验证了此行为和 journal。只有实际重新取得的 Job 确认没有活动进程，才使用正常 Exited 终态。OS reboot、conhost 与失去 Job 的完整树证明仍未完成。见 [恢复独立证据](../evidence/recovery-independent-final.md)。

后续增量：schema 3 逐成员 Runtime 身份已实现，双向 mixed GUI 树/root live 或 exit 每例两次真实恢复共四例通过；旧 schema 2 兼容/保守拒绝三例通过。未知 conhost companion 仍 Lost，OS reboot/NoJob 仍未验；见 [混合恢复证据](../evidence/mixed-recovery.md)。
