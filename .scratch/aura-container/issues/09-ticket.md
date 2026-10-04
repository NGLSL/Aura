# 09: 独立 Supervisor 启动与认证控制

Stage: P2
Status: claimed
Blocked by: [02: 真实 Runtime 身份与授权握手](02-ticket.md)、[07: 不可变 Run 快照与 Profile 变更处理](07-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** GUI/CLI 可按需连接当前用户与完整性级别的唯一后台 Supervisor，管理请求经过真实身份和对象所有权认证。

## 负责模块与契约

Supervisor、IPC 与客户端连接。管理通道与 Runtime bootstrap 授权分离；generation 与版本握手区分旧连接。

## 不包括

不安装 Windows 服务、不注册自启动、不自动提权；本票不接管 Job 或恢复实例。

## 验收标准

- [ ] 首次连接隐式后台启动无可见窗口，重复/并发连接收敛为当前管理范围一个有效 Supervisor。
- [ ] 同用户不同完整性级别或其他用户不能越权管理；远程客户端拒绝。
- [ ] 伪造报文 PID/Container/Instance 和 Runtime 发 Stop/Delete 拒绝且状态不变。
- [ ] 协议版本或 generation 不符明确拒绝/重连，不把陈旧响应用于新请求。
- [ ] 客户端断开不结束 Supervisor；连接失败和认证失败可区分。

## 验证证据

本地隐藏进程、并发启动及身份负向夹具；记录权限受限测试缺口。

## 关联验收

A04、A10、F06。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
