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

2026-10-06 最终 v2 增量：同一 x64 CLI 分别实际注入 x64 Probe 与 x86 Probe，strict Profile 单一 DoT IPv4 上游的 A/W/UTF8/Ex/async 五入口均 status 0、records 1、exit 0；每架构只运行一次，无自动重试。fresh Host 与 controller Runtime modules 为空，live DLL path/hash 对应最终 v2 pair。唯一 evidence JSON 为 `2b2a6cf2567b47b6b874547c9b286b7c` 与 `746ea8ca4ba14aa785591039983aa6dc`；旧 V3 timeout 和重试保留，不冒充最终结果。完整流量、其他 OS/缓存及长期验收仍未通过，票不关闭。见 [DoT 独立证据](../evidence/dot-independent-final.md)。

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。

2026-10-06 增量：默认系统信任、IPv4 公共 Cloudflare DoT transport 在 x64/x86 均取得正向样本。真实 strict Profile 注入脚本已记录 fresh Host、实际 live Runtime path/hash、完整原始输出与单次结果；加强后的单次运行 x64 五个 API 成功，x86 同步四个成功但 async timeout `1460`，保留失败且没有自动重试。旧 V3 的历史重试正向不代替本次单次结果或后续 async DLL 回归。系统级辅助流量观测、其他 OS/缓存与长时间矩阵仍未完成。见 [DoT 独立证据](../evidence/dot-independent-final.md)。
