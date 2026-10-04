# 45: 可定位审计与数据操作产品界面

Stage: P7
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** GUI/CLI统一展示真实保证、失败原因及clone/reset/export/import的范围和结果。

Blocked by: [39](./39-ticket.md)、[40](./40-ticket.md)、[41](./41-ticket.md)、[42](./42-ticket.md)、[44](./44-ticket.md)

## 负责模块与契约

GUI/CLI数据操作、审计查询与能力产品呈现。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不默认记录文件内容/凭据/完整DNS域名，不另造GUI业务路径，不展示未经证明保证。

## 验收标准

- [ ] 克隆、重置、删除、导入导出通过同一公开后端，停机/引用阻塞和恢复错误GUI/CLI一致。
- [ ] 数据清除和共享引用风险有具体清单，用户明确操作范围；取消不产生修改。
- [ ] 审计可按Container/Instance/generation及时间定位启动、拒绝、故障、策略变化和数据事务，不以PID唯一认领。
- [ ] 能力视图准确展示存储作用域、共享例外、网络/DNS区别、支持版本和Partial/Unsupported原因。
- [ ] 安装/升级/重启所需状态与失败恢复可理解展示，已停/TrackingLost与成功Running不可混淆；旧Compatibility使用无回归。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F07, F10（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
