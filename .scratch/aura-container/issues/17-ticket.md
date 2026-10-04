# 17: DoT 传输及 TLS 校验

Stage: P3
Status: claimed
Blocked by: [15: 统一任意 QTYPE Query Engine 与 UDP/TCP](15-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** Profile 可通过显式 literal IP 的 DoT 上游解析任意 QTYPE，验证 TLS 身份并按配置顺序处理失败。

## 负责模块与契约

Runtime DoT transport 与 Schannel，DNS TCP framing、连接生命周期和配置能力报告。复用 Query Engine deadline/cancel。

## 不包括

不跳过证书校验，不用宿主解析 server_name，不补入 UDP fallback或新系统证书。

## 验收标准

- [ ] literal IP 连接且按 server_name/SNI 或 IP SAN 验证身份；TLS 至少 1.2。
- [ ] 不可信、过期、名称不符和握手失败返回错误，只尝试显式后续上游。
- [ ] 半包、截断、多响应和最大长度正确处理，任意 QTYPE 使用统一 Query Engine。
- [ ] 超时与取消回收 socket/TLS 资源，不跨越总 deadline；复用池容量与空闲边界明确。
- [ ] 本地信任与吊销策略不产生隐式 Host DNS；x64/x86 实际注入均有证据。

## 验证证据

专用 TLS/DNS fixture、证书负向集和抓包；记录吊销辅助流量、连接复用及资源边界。

## 关联验收

F03。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
