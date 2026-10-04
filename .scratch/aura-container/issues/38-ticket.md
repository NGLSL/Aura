# 38: Container启动事务与组件故障不降级

Stage: P6
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 把已证明的文件、Registry、网络和对象能力变成一个原子启动与故障契约。

Blocked by: [34](./34-ticket.md)、[35](./35-ticket.md)、[36](./36-ticket.md)、[37](./37-ticket.md)、[12](./12-ticket.md)

## 负责模块与契约

Supervisor事务、可信策略绑定、Runtime能力聚合和故障验收。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不以配置启用代替真实能力，不切Compatibility或Host作为失败fallback。

## 验收标准

- [ ] 在应用入口放行前完成实际身份及全部必需backend门控；遵守03已证明loader/入口时序，不等待初始挂起线程ACK。
- [ ] 任何阶段失败只回滚本次新建资源/进程，不误杀已有host/A/B，不留下未受控活动应用。
- [ ] Runtime、Supervisor、可信通道、驱动与backend故障分别采用验证过的冻结/拒绝/终止行为，受保护I/O不能落宿主。
- [ ] 子进程归属、caller-requested suspended、并发启动/Stop及PID复用在同一事务契约中有确定结果。
- [ ] 恢复必须重验真实身份、策略摘要与控制能力；无法接管标TrackingLost并阻止该Container新Run，独立B继续可用。
- [ ] 聚合能力真实显示缺失原因，声明内Partial/Unverified阻止Container启动，保留Compatibility独立行为。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F01, F02, F04, F05, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
