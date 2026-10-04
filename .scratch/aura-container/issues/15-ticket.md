# 15: 统一任意 QTYPE Query Engine 与 UDP/TCP

Stage: P3
Status: claimed
Blocked by: [14: 有序 typed DNS 配置全链及迁移](14-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 所有现有 DNS API 适配共享任意 QTYPE Query Engine 和有序路由，保留已经实现的全 QTYPE 与 UDP/TCP 回归。

## 负责模块与契约

Runtime DNS packet、record、transport 与适配器。先核对已有修复，把路由、总 deadline、结果分类放到统一边界，不重复造已存在能力。

## 不包括

不新增任意 RR 缓存、不实现 DoT/DoH、不扩大到应用自带解析器。

## 验收标准

- [ ] A/AAAA/HTTPS/SVCB/TXT/PTR/SRV/CNAME/NS/root/未知 QTYPE 共用同一报文路径，native record/free 正确。
- [ ] UDP 截断在同一上游 IP/端口 TCP 重试；TCP_ONLY 和自定义端口遵循请求与配置。
- [ ] 响应来源/ID/Question/QR/opcode 检查、压缩和有界 CNAME、畸形或超大报文保持正确失败。
- [ ] NXDOMAIN/NODATA 为最终结果；连接错误、SERVFAIL/REFUSED按配置顺序重试且共享总预算。
- [ ] strict 下所有已支持路径上游失败返回解析失败；Host/non-strict 仅按明确模式工作。

## 验证证据

复用当前 DNS 实际注入夹具及既有绿色证据，新增仅覆盖统一路由新增差异；保存 host 零请求负向日志。

## 关联验收

A16、F03。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
