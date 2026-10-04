# 27: NTFS fixture可写打开COW原型

Stage: P5
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 限定NTFS夹具中host/A/B看到lazy host view，但第一次可写打开形成各自私有对象。

Blocked by: [26](./26-ticket.md)、[22](./22-ticket.md)

## 负责模块与契约

文件backend原型、身份/策略适配及fixture Probe。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不覆盖全卷，不扩大到真实用户数据，不以路径Hook自证内核保护。

## 验收标准

- [ ] host初始文件被A/B只读看到，A第一次可写打开copy-up后修改不改变host或B；独立宿主读取验证。
- [ ] 私有对象优先于host，尚未copy-up的host变化可见并明确lazy语义，不宣称冻结快照。
- [ ] 并发可写打开只产生确定私有backing，文件对象/handle绑定归属而非后续当前线程PID。
- [ ] 创建、读取、长度变化、共享模式和错误语义在夹具中与契约一致，copy-up失败不写host。
- [ ] 隔离环境停止/重开后私有数据保留，清理仅验证过的fixture根；返回支持限制和原始系统错误。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
