# 02: 真实 Runtime 身份与授权握手

Stage: P0
Status: claimed
Blocked by: [01: 启动失败句柄和资源清理](01-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 启动结果报告实际加载的 Runtime、不可变配置身份和必要 Hook 能力；非 owner 或伪造身份不能绑定实例。

## 负责模块与契约

Runtime、Broker bootstrap、Launcher 身份确认。以真实 Windows 客户端身份、PID 与创建 generation 绑定 Instance/Profile；控制权限与 bootstrap 权限分离。

## 不包括

不提供 Supervisor 寿命或完整入口门控；握手支持已执行 bootstrap 的受控夹具，不承诺初次 Resume 前 ACK。

## 验收标准

- [ ] 实际模块路径/hash、Runtime 版本、Instance/Profile/配置摘要与请求一致才确认。
- [ ] 重复 LoadLibrary 或环境变量标记不能替代实际身份；同 PID 不同 Profile/Instance 拒绝绑定。
- [ ] 伪造 PID、generation、Profile、非 owner 和远程 bootstrap 均拒绝且不修改状态。
- [ ] 目标 Runtime 不能执行管理命令或领取其他进程的快照。
- [ ] 必要 Hook 缺失、协议不兼容或握手超时返回具体失败，不显示完整成功。

## 验证证据

实际注入 Probe 与非注入控制；保存握手事实、冲突及认证负向日志，覆盖 x64/x86 DTO。

## 关联验收

A02、A07、A10、F01、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
