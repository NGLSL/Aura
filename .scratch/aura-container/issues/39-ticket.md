# 39: 正式Container GUI/CLI启用与支持矩阵

Stage: P6
Status: ready-for-agent
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 用户可显式选择正式Container并看到真实支持范围，旧Compatibility入口仍可用。

Blocked by: [38](./38-ticket.md)、[08](./08-ticket.md)

## 负责模块与契约

GUI/CLI产品入口、领域状态呈现和支持矩阵验收。

保持总规格的稳定Container/Instance身份、不可变Run快照和host/A/B独立对照。修改限定于本票职责；保留其他Agent改动。关键契约变化或未证明机制需报告证据，由主Agent决定，不静默降级。

## 不包括

不默认升级旧环境，不宣传任意Windows兼容或安全边界，不把V1视作最终完成。

## 验收标准

- [ ] GUI/CLI都从同一事务启动Container，能力不足返回一致原因，不静默退Compatibility。
- [ ] 普通支持Win32目标可选Profile及存储/网络/对象策略并查看实际运行身份，A/B配置和私有数据彼此独立。
- [ ] 共享可写、lazy host读取、限定NTFS/HKCU及HTTPS/服务边界清晰展示，不暴露无用内核实现细节。
- [ ] Packaged/AppContainer/高完整性/不支持broker目标明确拒绝或保留旧入口说明；原应用sandbox保持。
- [ ] 支持矩阵关联真实测试版本、verified/unsupported/unverified和证据；运行成功不能替代全能力验证。

## 验证证据

经公共CLI、真实注入/内核Probe或本票指定的资格入口验证可观察行为，保存配置快照、系统/组件版本、模块路径与hash、host/A/B原始结果和失败原因。需要驱动、故障或危险系统配置的验证只在明确隔离测试环境执行；本票不授权宿主安装、外部申请或发布。静态检查、fixture、隔离环境、实际安装与真实应用证据分别列出；未执行、缺材料、权限不足及skip不得算通过。文档预检票保存可复现检查与来源，不伪造运行证据。

最终验收关联：F01, F09, F10（见[实施规划最终验收](../implementation-plan.md#最终验收)）。
