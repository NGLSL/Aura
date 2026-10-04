# 36: 正式Session网络策略与DNS出口集成

Stage: P6
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 正式Container启动时网络策略和DNS出口同时受可信归属控制，GUI退出后继续生效。

Blocked by: [25](./25-ticket.md)、[20](./20-ticket.md)、[10](./10-ticket.md)

## 负责模块与契约

正式网络backend、DNS配置策略适配、Supervisor启动/恢复与流量验收。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不合并strict DNS与任意HTTPS保证，不支持不能归属的服务代执行应用。

## 验收标准

- [ ] Host/Deny/Allowlist与四种DNS transports来自不可变快照，A/B和host同exe配置分别正确。
- [ ] 策略在应用连接前成立，子进程与PID复用无窗口；明确loopback、IPv6/QUIC、直接IP和入站支持。
- [ ] 严格DNS上游/引导/TLS/取消失败不Host fallback，53/853只按配置出口允许，未配置公共服务器不出现。
- [ ] GUI断开、Supervisor/DNS组件故障与重连保持25已证明策略；后端缺失拒绝启动，不降级Host。
- [ ] 正式审计和能力说明准确区分网络规则、API DNS及应用DoH边界，代执行不能归属的应用被拒绝或不受支持。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F03, F04, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
