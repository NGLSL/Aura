# 29: 映射写、paging、SQLite/WAL与崩溃原型

Stage: P5
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 映射写和SQLite/WAL工作负载落到私有backing，崩溃恢复仍保持host/A/B隔离。

Blocked by: [27](./27-ticket.md)

## 负责模块与契约

文件section/paging绑定、持久恢复原型和受控工作负载。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不按paging当前PID路由，不将普通WriteFile通过扩大为映射写通过。

## 验收标准

- [ ] 可写section建立前copy-up，映射写/flush/解除映射与paging I/O均基于已绑定私有对象。
- [ ] host/A/B同时映射同初始文件，A修改及重开只改变A；独立host内容/hash不变。
- [ ] SQLite/WAL的主库、WAL、SHM、锁和原子保存共同满足A/B独立事务与重启一致性。
- [ ] 故障注入覆盖copy-up、映射持有、flush与提交窗口，恢复无host回写、破坏另一环境或暴露半提交私有状态。
- [ ] 活动section及文件对象未释放时删除/清理被拒绝或安全延期；记录已支持组合和拒绝项。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02, F06, F07（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
