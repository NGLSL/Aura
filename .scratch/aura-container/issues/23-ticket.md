# 23: WFP按进程Host/Deny网络原型

Stage: P4
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 同一程序在host/A/B分别执行Host或Deny规则，展示宿主与其他环境互不影响。

Blocked by: [22](./22-ticket.md)

## 负责模块与契约

网络backend原型、身份策略绑定、网络Probe和隔离测试配置。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不提供网络namespace，不全局封禁服务，不把exe路径当容器身份。

## 验收标准

- [ ] 独立host、A Host、B Deny同时访问本地受控端点，正向及拒绝结果均与各自策略匹配。
- [ ] 至少覆盖IPv4/IPv6、TCP建立、UDP发送及loopback；未覆盖入站/监听明确记录并拒绝超出能力请求。
- [ ] 网络归属来自22可信绑定，PID复用、退出清理及同exe不同路径不改变其他实例行为。
- [ ] 拒绝返回可定位原因与策略摘要，审计不默认暴露载荷或凭据。
- [ ] Supervisor断连和驱动不可用按已声明安全状态处理，已有Deny不悄悄变Host；记录流量与宿主独立观察。

## 验证证据

2026-10-06 增量：[IPv4 WFP 源码、SYS 构建和共享决策 fixture](../evidence/service-wfp-followup.md)。ALE_AUTH_CONNECT_V4 注册与 Host/Pending/Deny 分类已实现并通过非加载验证；未安装或加载驱动，没有实际包过滤证明。本轮按用户要求仅推进 IPv4，IPv6 后置且不阻塞这批源码交付；完整双栈网络保证仍须另行验收，不关闭本票。

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F04, F06（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
