# 32: Registry枚举、通知、Native/WOW64与句柄矩阵

Stage: P5
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 声明HKCU作用域中的Win32和Native入口、枚举与通知都看到同一隔离视图。

Blocked by: [31](./31-ticket.md)

## 负责模块与契约

Registry入口/对象资格测试与必要原型补齐。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不靠新增用户态入口数量宣称全覆盖，不隐含支持事务/COM注册等未验证能力。

## 验收标准

- [ ] key/value合并枚举、数量信息、多值读取、小buffer及恢复遍历与实际union一致，tombstone不泄露host项。
- [ ] Win32和Native读写/删除从同fixture得到一致结果，x64/x86与WOW64视图分别真实验证。
- [ ] 通知对私有变化和允许的lazy host变化遵循明确契约，host/B通知不会误认A私有修改。
- [ ] 继承、复制和跨边界Registry handle逐项支持或拒绝，不能用于写host或B；关闭/退出后引用可靠释放。
- [ ] Registry ACL、链接、事务及不支持COM场景记录原始错误与明确拒绝；崩溃/重开后视图一致。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
