# 37: 命名对象、单实例与broker边界

Stage: P6
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 同应用host/A/B不会借单实例或IPC将请求静默交给另一环境，支持与拒绝路径可复现。

Blocked by: [05](./05-ticket.md)、[22](./22-ticket.md)

## 负责模块与契约

对象作用域原型到正式策略、启动冲突处理与真实应用资格矩阵。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不任意重写系统IPC，不关闭应用sandbox，不注入全局服务补覆盖。

## 验收标准

- [ ] named mutex/event、共享内存和named pipe的声明对象范围在host/A/B分别验证互不误用，保留应用所需同Container共享。
- [ ] 实际单实例应用的启动及请求转交不能让A请求在host/B执行；无法满足时拒绝支持该应用，不记录假新实例。
- [ ] ALPC/COM/Shell/WMI/共享服务分别记录代执行与数据跨界结果，无法可信归属或隔离使完整Container目标不受支持。
- [ ] 对象规则来自可信快照且授权有效，目标不能认领他Container对象或更改策略；退出/PID复用正确清理。
- [ ] 真实多进程/浏览器评估保持sandbox，未证明renderer Environment View与IPC不标Verified；输出正式支持边界。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F05, F06, F09（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
