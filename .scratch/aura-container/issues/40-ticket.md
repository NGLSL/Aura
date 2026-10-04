# 40: 停机克隆与新UUID

Stage: P7
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 已停止Container可克隆为新身份，私有文件与Registry状态复制而共享目录保持显式引用。

Blocked by: [39](./39-ticket.md)

## 负责模块与契约

领域/数据事务、CLI克隆入口及host/A/B数据验证。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不提供在线快照，不复制宿主fallback或共享目录内容，不沿用运行身份。

## 验收标准

- [ ] 活动/TrackingLost、section/hive/策略引用未释放时拒绝克隆，保持源Container不变。
- [ ] 新UUID、独立管理根和运行generation生成，配置/私有文件/Registry/tombstone一致复制，源与克隆可独立修改。
- [ ] 共享目录保持引用并输出共享可写风险，不把lazy view克隆称完整host快照。
- [ ] 复制失败、磁盘满和崩溃不发布半成品Container，临时清理不越过自己的验证根。
- [ ] 停机克隆后重启两环境与独立host实际验证数据、Profile引用和策略，不重用旧PID或内核归属。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F07（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
