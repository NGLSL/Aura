# 13: 活动 Runtime bundle 保留及控制协议升级

Stage: P2
Status: claimed

2026-10-06 增量：bundle retention 在最终稳定 lease handle 单次读取摘要，使用恢复共享 cooperative deadline；拒绝网络/device/relative/reparse 路径并重验实际 file identity。实际 junction、写/delete 共享拒绝和慢读超时测试通过；同步 Windows I/O 不可硬取消的边界保留，见 [本轮证据](../evidence/nonvm-completion-followup.md)。
Blocked by: [12: Supervisor 崩溃恢复与 TrackingLost](12-ticket.md)、[08: 工作区运行和 GUI/CLI 能力展示](08-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 升级或切换后端前确认活动实例兼容性，保留它们需要的 Runtime，无法安全接续时明确拒绝。

## 负责模块与契约

Runtime bundle 生命周期、Supervisor/客户端协议与升级协调。按实际模块和版本引用计数，旧实例继续使用不可变快照。

## 不包括

仅实现和验证升级准备/本地夹具；不授权发布、安装部署或删除用户活动数据。

## 验收标准

- [ ] 旧实例运行时新版本客户端重连需版本协商，不能用新 DTO 错读旧状态。
- [ ] 仍被活动或 TrackingLost 实例引用的 Runtime bundle 不删除或覆盖。
- [ ] 不兼容 Supervisor 更新拒绝接续并说明处理方式，不静默降级能力。
- [ ] 旧实例退出并证明资源释放后才可回收对应 bundle；宿主和 B 引用不受 A 清理影响。
- [ ] 本地旧/新版本升级 fixture 保留配置和运行身份；GUI/CLI 能力原因一致。

## 验证证据

两版本本地控制协议与实际加载模块/hash记录、活动引用清理负向测试；实际安装升级另外验收。

## 关联验收

A14、A15、F06、F08。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。

2026-10-06 增量：Supervisor 对活跃及已恢复实例保留 Runtime bundle 的稳定只读句柄，仅允许 read sharing；从句柄计算有界 SHA-256，核对已认证摘要与 Windows file identity，拒绝路径替换、reparse 和篡改。实际测试证明持有期间写入/删除被拒绝，释放后修改内容再获取被拒绝；确认 Job 为空或 Stop 完成后释放 lease。Supervisor 崩溃后的外部 cleanup、真实 installer upgrade 与 OS reboot 仍未验，不能从句柄保留测试推断这些场景已通过。见 [独立证据](../evidence/recovery-independent-final.md)。
