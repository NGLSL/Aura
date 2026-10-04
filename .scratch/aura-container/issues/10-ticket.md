# 10: Supervisor 持有 Job 并完成启动事务

Stage: P2
Status: claimed
Blocked by: [09: 独立 Supervisor 启动与认证控制](09-ticket.md)、[03: 受控 bootstrap 及应用入口门控](03-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** Supervisor 持有实例 Job 和完整快照，在门控确认后发布运行；GUI 退出不销毁运行监管。

## 负责模块与契约

Supervisor、Launcher、Job 与 Runtime bootstrap。事务唯一 request ID、资源 owner、Run 状态和持久身份一致。

## 不包括

不实现崩溃恢复、不以 GUI 句柄维持 Job，不自动终止所有 GUI 退出后的应用。

## 验收标准

- [ ] 启动验证、创建、Job、门控与确认全部完成才发布 Running。
- [ ] GUI 在启动不同阶段退出/断连不留下未纳管进程；请求结果可按唯一身份查询。
- [ ] 注入/Job/确认/恢复失败仅回收本次资源，原有实例不受影响。
- [ ] Supervisor 持有 Job 和进程身份，GUI 全部关闭后目标及受支持子进程继续运行。
- [ ] 重复 Run 请求返回同一事务结果，不重复启动应用或重新绑定已有 PID。

## 验证证据

启动阶段故障注入与 GUI 断连；独立检查 Job、进程和模块身份。

## 关联验收

A04、A06、A07、F01、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
