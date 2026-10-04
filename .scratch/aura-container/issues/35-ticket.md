# 35: 正式应用Registry backend

Stage: P6
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 受支持应用的限定Registry子树按Container持久独立，读写删除和枚举跨入口一致。

Blocked by: [33](./33-ticket.md)、[10](./10-ticket.md)

## 负责模块与契约

正式Registry backend、快照/生命周期对接及公开启动验收。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不整体替换系统根，不自动把未配置HKLM列为支持，不修改宿主环境配置。

实施边界：复用33选定且资格通过的kernel Registry backend，把限定fixture扩展为正式应用子树与生命周期路径；若需要替换核心机制，报告并另行拆票，不在本票从零重造整套驱动。

## 验收标准

- [ ] 正式应用HKCU作用域的host/A/B读写、删除及数量/枚举符合契约，独立宿主hive检查不变。
- [ ] Win32/Native/x64/x86/WOW64和多进程同Container以32已证明契约运行，拒绝未支持调用。
- [ ] 私有hive/存储、tombstone和通知重启后可靠恢复；同Container实例共享状态，B独立。
- [ ] 句柄引用与Stop、崩溃、backend失联关联，未释放不提前删除私有状态，故障不Host回写。
- [ ] 明确共享例外与必要HKLM子树的单独能力，GUI/CLI能力来源于实际backend报告而非配置勾选。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F01, F02, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
