# 30: 路径别名、跨边界句柄与ACL拒绝矩阵

Stage: P5
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 用路径与handle绕过案例验证限定文件作用域的真实边界，不能隔离的操作明确拒绝。

Blocked by: [27](./27-ticket.md)、[29](./29-ticket.md)

## 负责模块与契约

文件授权/对象绑定资格矩阵及host/A/B对照客户端。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不修改宿主ACL制造通过，不允许未验证路径自动宿主写入。

## 验收标准

- [ ] hardlink、ADS、open-by-ID、junction/reparse、大小写和最终路径逐项报告支持或拒绝，等价对象授权一致。
- [ ] 继承及DuplicateHandle的host→A、A→B和A→host可写句柄被拒绝或有已证明隔离行为；测试实际写和映射。
- [ ] 跨卷rename、网络卷、管理根直访及重定向循环不越过规则；外部fixture哨兵不变。
- [ ] ACL拒绝、共享冲突及权限不足保留原始错误，不提升权限、关闭安全功能或误记为成功。
- [ ] 并发路径替换/reparse竞争在拒绝规则内无host写入；报告声明范围与必须阻塞正式backend的缺口。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
