# 04: 受支持子进程传播与身份确认

Stage: P0
Status: claimed
Blocked by: [03: 受控 bootstrap 及应用入口门控](03-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 普通受支持 Win32 子进程继承相同不可变配置，并在入口放行前确认身份；失败不伪装成完整进程树。

## 负责模块与契约

Runtime 进程创建适配、Launcher/Job 与 bootstrap。每个子进程使用真实 generation 和父实例身份，通知成功不等于绑定成功。

## 不包括

不扩大到 WMI/COM/服务代执行或 Chromium sandbox renderer；不修改宿主启动路径。

## 验收标准

- [ ] CreateProcess A/W 和已支持 AsUser 路径分别验证子进程 Profile/Instance/Container 身份。
- [ ] cmd/PowerShell 中间进程与 x64/x86 混合子树保持相同快照且 Job 归属正确。
- [ ] 注入、身份或子进程门控失败时仅清理本次子进程，不遗留未受控进程。
- [ ] 父子启动与绑定并发、重复通知不产生双重认领或错误 Profile。
- [ ] 不支持路径报告实际 Partial/Unsupported 原因；Job 成员不自动视作 Runtime 已验证。

## 验证证据

实际父子 Probe 输出与独立 Job/模块观察；包含失败注入与并发样本。

## 关联验收

A02、A06、A08、F01、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
