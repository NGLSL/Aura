# 16: 严格解析所有入口与异步取消

Stage: P3
Status: claimed
Blocked by: [15: 统一任意 QTYPE Query Engine 与 UDP/TCP](15-ticket.md)、[04: 受支持子进程传播与身份确认](04-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 严格模式下所有声明支持的 Windows resolver 入口和父子进程都使用完整 Profile DNS，失败、异步取消及不支持输入不会回退宿主。

## 负责模块与契约

Runtime resolver 适配器、Hook 能力门禁与 snapshot 恢复。包含 DnsQuery A/W/UTF8/Ex、getaddrinfo、GetAddrInfoW/Ex A/W 的同步和异步契约。

## 不包括

不拦截应用自带 DoH/DoT/DoQ，不实现 WFP；numeric/localhost 仅在纯本地契约内直通。

## 验收标准

- [ ] 各声明入口的成功、超时、失败与异步调用实际注入验证；strict 的 Host DNS fixture 零请求。
- [ ] IDNA、版本/options/interface/caller server list 不支持时显式错误，不能将网络查询交还 Windows。
- [ ] 纯 numeric/localhost 不发网络；强制 wire 选项不能借本地路径触发宿主。
- [ ] 取消后不继续下一上游，回调最多一次，无 use-after-free/句柄泄漏，总 deadline 覆盖请求。
- [ ] 必要 Hook 或完整 snapshot 缺失拒绝启动/解析；后台不可用只使用同一完整快照，不默默改 Host。

## 验证证据

所有 API 的本地成功/失败/取消夹具与父子实际注入记录；并发与资源稳定及 Host 对照。

## 关联验收

A02、A06、A16、F03、F06。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。


## 当前实施记录

2026-10-06：补充 ExW event/callback/cancel Runtime-owned worker，复制 caller event，提交返回 997、pending Internal 10036、原子最后发布 terminal 状态；原生 status-only helper 保持重复读取语义。DnsQueryEx 使用加锁的 generation token，完成不再回写 caller storage，callback 内旧 token 与重入后的 stale copy 均本地返回 87，不能转发 Host 或取消新 generation。Profile 下 ExA async、unsupported provider/namespace/flags 明确拒绝。冻结 v2 x64 全 DNS CLI 32 项及 fresh Host 最终 workspace 395 项通过，完整系统流量与所有错误输入/资源矩阵仍未闭，票保持 claimed。详见 [Resolver 独立证据](../evidence/resolver-async-final.md) 和 [最终检查](../evidence/implementation-progress.md)。

2026-10-04：已实施并验证可独立交付的切片；完整验收尚未全部通过，本票未关闭。实际运行、静态检查、失败来源和剩余缺口见 [实施进度](../evidence/implementation-progress.md)。不得从 claimed 或某组测试通过推断整票/完整 Container 完成。
