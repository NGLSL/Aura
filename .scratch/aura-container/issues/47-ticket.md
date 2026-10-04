# 47: 故障并发、Verifier/HVCI与filter interop资格验证

Stage: P8
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 在可恢复隔离环境证明故障、竞争和其他过滤组件下不会误归属、误杀或退回宿主。

Blocked by: [44](./44-ticket.md)、[38](./38-ticket.md)

## 负责模块与契约

内核/生命周期资格验证、故障注入和兼容报告。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不在宿主启用Driver Verifier，不关闭HVCI/Secure Boot，不将蓝屏恢复当行为通过。

## 验收标准

- [ ] 隔离环境执行Driver Verifier及声明Secure Boot/HVCI配置，保存设置、结果、dump/错误及恢复流程。
- [ ] 创建/绑定/Resume/Stop/退出、PID复用与并发多Container压力无未保护窗口、跨环境规则或handle生命周期错误。
- [ ] Supervisor/Runtime/DNS/驱动/安装升级故障分别验证不静默降级，故障仅处理已确认归属目标。
- [ ] 与声明支持的安全软件/文件filter组合真实验证关键I/O、网络和安装卸载；缺环境保持Unverified。
- [ ] OS恢复与数据重开后host/A/B完整性独立验证，任何崩溃或声明内缺陷阻塞最终资格而非记已完成。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F02, F04, F06, F08（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
