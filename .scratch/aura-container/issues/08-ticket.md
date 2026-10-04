# 08: 工作区运行和 GUI/CLI 能力展示

Stage: P1
Status: claimed
Blocked by: [07: 不可变 Run 快照与 Profile 变更处理](07-ticket.md)、[03: 受控 bootstrap 及应用入口门控](03-ticket.md)、[05: 进程入口和真实应用覆盖矩阵](05-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 从持久工作区启动受支持应用，GUI/CLI 显示同一实际身份、注入时机及能力原因。

## 负责模块与契约

GUI/CLI、Launcher 与实例模型。工作区入口要求继承、普通 Win32 Executable/Command；共享已有启动验证。

## 不包括

不提供后台重连；不支持 Packaged、brokered console、AppContainer、高于管理范围目标。

## 验收标准

- [ ] A/B 运行同一 Probe，各自实际配置及身份匹配；宿主控制保持原值。
- [ ] Packaged、不支持 console、inherit-off 和权限超出范围明确拒绝，不偷偷改启动方式。
- [ ] 旧直接 Application/Profile 入口仍可用，但不能绕过同 PID 不同身份冲突检查。
- [ ] GUI/CLI 一致显示 Verified/Partial/Unsupported/Unverified 与原因，Loaded 不等于全部能力。
- [ ] 启动失败保留错误且不创建 Running 假记录；尚未实现存储隔离和 transports 不显示可用。

## 验证证据

实际工作区运行和旧入口回归；GUI/CLI 同一实例对照、 unsupported 请求负向记录。

## 关联验收

A02、A07、A13、A14、F01、F10。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
