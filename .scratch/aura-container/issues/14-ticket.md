# 14: 有序 typed DNS 配置全链及迁移

Stage: P3
Status: claimed
Blocked by: [07: 不可变 Run 快照与 Profile 变更处理](07-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 用户保存排序的 UDP/TCP/DoT/DoH 上游及 strict，完整配置贯穿工作区快照和 Runtime；旧 IP 列表按原顺序迁移。

## 负责模块与契约

Core/Storage、GUI/CLI、Broker/Supervisor 和 Runtime DTO。保持 Rust 控制面/C++ 数据面；版本化完整快照，URL 不塞进 IP 字段。

## 不包括

不实现加密传输、不自动添加公共或明文 DNS、不启用未实现能力。

## 验收标准

- [ ] UDP/TCP literal IP 与端口、DoT 证书身份、DoH https/bootstrap 数据完整 roundtrip，顺序保持。
- [ ] 旧 servers 迁移为原顺序 UDP/53；混合旧新字段、无效端口或缺 bootstrap 明确报错。
- [ ] 新建及迁移 VirtualView 默认 strict；Host 和显式 non-strict 独立保存并说明失败行为。
- [ ] GUI 可排序且显示明文 fallback，CLI 旧 IP 简写兼容；未完成的 transport 不能运行时误称可用。
- [ ] IPC/env 超限、过滤、字段丢失和版本损坏拒绝启动/解析，不切 Host。

## 验证证据

旧配置 fixture、GUI/CLI 排序 roundtrip、x64/x86 DTO和损坏/边界记录。

## 关联验收

A03、A16、F03、F10。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
