# 46: Host/A/B实际应用完整矩阵

Stage: P8
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 以真实应用证明声明Container支持范围，而不是仅靠Probe或模拟程序发布保证。

Blocked by: [45](./45-ticket.md)

## 负责模块与契约

实际应用行为资格矩阵、独立宿主控制和原始证据归档。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不预先保证浏览器合格，不把注入进程观察当宿主事实，不反复无界唤起GUI。

## 验收标准

- [ ] 矩阵至少包含受控Probe、普通Win32 GUI、多进程应用、SQLite/WAL工作负载及实际浏览器评估，记录版本/位数/系统。
- [ ] 每个支持应用在host/A/B独立执行配置、文件、Registry、网络和单实例行为，模块路径/hash和快照身份可核验。
- [ ] 关闭GUI、后台重连及正常重启后实际应用状态符合契约，范围内host内容独立读取证明无越权写入。
- [ ] 浏览器保持sandbox，renderer/服务/IPC不能满足时列Unsupported并拒绝完整Container；不以三份Probe替代。
- [ ] 每个声明支持项关联正负向证据；权限不足/未执行保留原始错误及Unverified，不能以skip通过资格。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F01, F02, F03, F04, F05, F09（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
