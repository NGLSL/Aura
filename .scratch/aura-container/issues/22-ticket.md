# 22: 内核进程归属与可信策略通道原型

Stage: P4
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 在隔离环境让内核可区分同一exe的host/A/B，并仅接受可信控制面绑定不可变策略。

Blocked by: [21](./21-ticket.md)、[02](./02-ticket.md)、[03](./03-ticket.md)、[10](./10-ticket.md)

## 负责模块与契约

内核身份原型、Supervisor控制适配、受控进程Probe。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不以PID、环境变量或Runtime自报作信任根；不实现完整存储或网络过滤。

开始条件：21的报告交付不等于实验资格通过。必须实际具备已指定隔离环境、可用构建/测试签名及恢复路径才能开始加载实验；缺失时保留条件阻塞，不能从报告推定驱动可用。

源码准备的新增资格项（2026-10-06）：`PsSetCreateProcessNotifyRoutineEx` 的通知覆盖不能假定包含 Native process clone / PSS VA clone。adapter 对成员子进程的拒绝只覆盖收到通知的创建路径。克隆必须在隔离环境证明可信归属或实际拒绝，在此之前保持驱动无加载资格、Container/Strong 不启用；不能把未登记对象的 Host 分类当作完整进程树保证。见 [源码准备](../evidence/kernel-policy-preparation.md) 和 `drivers/envbox-policy/ADAPTER.md`。

## 验收标准

- [ ] host/A/B同exe保持不同身份，PID与creation generation复用后旧策略不命中新进程。
- [ ] 策略消息版本、长度、owner、请求身份及generation校验失败时不改变现有绑定；目标不能自认领任意PID。
- [ ] 创建/退出/绑定/撤销形成确定状态，应用受保护执行前归属成立；证明与03实际启动门控的对接时序。
- [ ] 子进程和caller-requested suspended路径分别给出原始身份及时序证据，未证明路径明确拒绝。
- [ ] 可信控制组件消失时已有绑定不会自动转成Host；安全卸载和资源释放在隔离环境有可恢复证明。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F01, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
