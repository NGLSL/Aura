# 34: 正式AppData/Temp与共享目录文件backend

Stage: P6
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 受支持应用在正式AppData/Temp和配置NTFS目录保存私有数据，共享例外按规则生效。

Blocked by: [33](./33-ticket.md)、[10](./10-ticket.md)

## 负责模块与契约

正式文件backend、Supervisor快照/生命周期对接、公开CLI与实际应用fixture。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不覆盖整个C:，不自动放行未知写入，不提供安全沙箱承诺。

实施边界：复用33选定且资格通过的kernel文件backend，把限定fixture扩展为规则化AppData/Temp与共享目录的正式端到端路径；若发现需要替换核心机制，报告并另行拆票，不在本票无界重写驱动。

## 验收标准

- [ ] 实际受支持应用的AppData/LocalAppData/Temp及显式目录A/B读写/删除/原子保存符合正式契约，host独立内容不变。
- [ ] 共享只读拒绝写入，共享可写允许并可见宿主影响，未匹配写入和管理根直访拒绝。
- [ ] 33所证明映射、WAL、枚举和handle边界在正式启动/停止链中保持，策略来自不可变Run快照。
- [ ] 文件backend缺失/失联时拒绝新Container启动，已运行目标采用已证明安全行为，不能回Host写入。
- [ ] 按Container UUID可靠持久化和释放引用，重开与多个同Container实例共享私有状态；提供实际模块/hash及支持限制。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F01, F02, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
