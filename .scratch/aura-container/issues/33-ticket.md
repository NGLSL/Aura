# 33: 文件与Registry集成门禁及正式backend决定

Stage: P5
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 给出有限存储原型的综合Go/No-Go及正式backend方案，使产品集成建立在完整行为证据上。

Blocked by: [28](./28-ticket.md)、[29](./29-ticket.md)、[30](./30-ticket.md)、[32](./32-ticket.md)

## 负责模块与契约

存储集成资格fixture、backend决定记录和支持范围。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不因单fixture通过承诺全卷或任意应用，不擅自改变已定义存储语义。

## 验收标准

- [ ] 同Run执行文件、映射/WAL与Registry操作，host/A/B独立观察均满足作用域契约与重开持久化。
- [ ] 列出28–32各必需案例、证据文件和系统/模块版本；missing或Partial不能当门禁通过。
- [ ] 评估minifilter/Registry callback/私有存储方案及一致身份、对象生命周期、故障事务，选定可交付backend并记录理由。
- [ ] 明确首个正式AppData/Temp及HKCU范围、拒绝操作和驱动拆分，无跨模块复制身份系统。
- [ ] 无法满足必需行为时保持阻塞并提交证据和替代路线供主Agent决定；不把总目标缩为V1。

## 验证证据

门禁：No-Go报告不解除34/35的依赖。本票选定正式kernel backend、驱动职责边界及可复用原型；下游沿已证明机制扩展到真实作用域，不在单票从零重造整套驱动。核心行为未证明则保持未完成/阻塞。

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02, F06, F08（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
