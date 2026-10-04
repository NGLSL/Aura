# 24: Allowlist协议、IPv6/QUIC及动态DNS策略原型

Stage: P4
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 按显式目的地址、端口和协议允许出口，并让域名派生地址具有可验证的生命周期。

Blocked by: [23](./23-ticket.md)、[14](./14-ticket.md)

## 负责模块与契约

网络Allowlist原型、DNS结果到策略适配及端点测试。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不添加公共DNS，不以已知DoH列表宣称阻止所有DoH，不放宽任意HTTPS。

## 验收标准

- [ ] IPv4/IPv6、TCP/UDP及QUIC案例分别验证允许匹配与地址/端口/协议不匹配拒绝，直接IP不能越过规则。
- [ ] loopback、入站及监听显式配置；未声明能力拒绝，不隐含无限本地访问。
- [ ] 域名地址仅来自可信配置的DNS结果，TTL、更新失败、撤销及旧连接处理规则明确且实际验证。
- [ ] DNS动态策略跨Container和generation隔离，A查询不能替B增加出口；更新与连接并发结果确定。
- [ ] 允许HTTPS明确展示应用DoH仍可能符合允许规则；只有受控目的地/代理的实际约束才报告对应保证。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F03, F04（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
