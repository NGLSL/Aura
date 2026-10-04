# 31: 限定HKCU私有union读写删除原型

Stage: P5
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 限定应用HKCU夹具中host/A/B独立读写，私有值和删除标记形成真实union视图。

Blocked by: [26](./26-ticket.md)、[22](./22-ticket.md)

## 负责模块与契约

Registry backend原型、身份/策略绑定及独立宿主Registry观察。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不整体重映射HKCU/HKLM，不把私有hive存在当union完成。

## 验收标准

- [ ] A/B初始读取host key/value，A创建/覆盖只影响私有状态，独立宿主和B仍保持原值。
- [ ] key/value删除、重建和tombstone遮蔽有一致语义，重启后删除项不从host复活。
- [ ] Registry对象与句柄绑定归属，不能仅以调用时PID决定写入；跨环境句柄先拒绝未证明操作。
- [ ] 未匹配写入和管理存储直访拒绝，共享例外必须显式；故障不落回宿主。
- [ ] 两进程同Container并发与A/B并发保存/重开可验证，保留类型、长度、错误和host真实hive证据。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
