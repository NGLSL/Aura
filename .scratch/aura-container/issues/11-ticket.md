# 11: GUI 重连与幂等 Stop/Stop all

Stage: P2
Status: claimed
Blocked by: [10: Supervisor 持有 Job 并完成启动事务](10-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 重开 GUI/CLI 后可查看仍运行的实例并停止自身工作区，重复请求和并发 Run 有确定结果。

## 负责模块与契约

Supervisor 管理 API、GUI/CLI 实例展示和 Job 停止。Stop 以实际 generation/归属为准；Stop all 明确命令快照与并发顺序。

## 不包括

不按 exe 名称停止进程、不恢复失去控制的实例、不提供数据清除。

## 验收标准

- [ ] 关闭重开 GUI 后实例身份和状态一致，当前 owner 可 Stop。
- [ ] 重复 Stop 返回已停止/已退出的幂等结果，不误停 PID 重用目标。
- [ ] Stop all 仅作用于确定目标集合；并发 Run 的加入规则明确且验证一致。
- [ ] 宿主同 exe、B 工作区及其他管理范围不受 A 的 Stop/Stop all 影响。
- [ ] 停止后 Job 与句柄回收、最终退出记录正确；GUI/CLI 查询一致。

## 验证证据

A/B/宿主同 exe 实际进程对照、并发命令日志与资源回收证据。

## 关联验收

A04、A11、F01、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
