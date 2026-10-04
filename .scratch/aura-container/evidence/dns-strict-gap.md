# Strict DNS 剩余入口与实施边界

Date: 2026-10-04
Status: partial-validation

本记录来自当前代码只读审查，属于票 16；控制流缺口与真实网络观测分别标记。已有非 ASCII 输入显式失败，不重复列为透传缺口。

以下五项控制流已实现，并扩展 Probe。x64 全 QTYPE/错误/取消/TCP 矩阵 28/28 通过；x86 strict 与 native Host/Host Profile 各 12 次调用通过。最终 bundle 刷新暴露的短命 Probe 身份确认竞态已通过 mandatory identity ACK 和精确 generation 的已确认退出缓存修复；双架构 Sleep(0) 各 5/5、退出缓存 2/2，V2 DNS 再验 28/28。Raw 首版是同步拒绝，不是支持 Raw 解析。W 同步 TIMEVAL 共用较短 deadline；A 的保留 timeout 参数明确拒绝。

## 必须修复的控制流

1. GetAddrInfoExA/W 在 Profile 模式判断之前把 async 参数及非 DNS namespace 交给 native。先处理明确 Host；strict 下尚未实现的 async、非 DNS namespace、非空 provider GUID 同步返回 WSAEOPNOTSUPP，不发布 pending、回调、事件或取消 token。
2. strict 空列表、超限、损坏或缺字段不能关闭路由或改写 dns_mode=Host。明确 Host、有效 strict、无效 strict 分开；无效配置拒绝 Runtime 初始化。完整 typed snapshot 与版本一致性由 14 接线。
3. 必需 DNS Hook 逐项确认；resolver/free、DnsQueryEx/cancel、当前 OS 存在的 Raw 阻断入口任一 attach 失败必须 abort Detours transaction。统计数量不能替代成功门禁。
4. 当前系统 dnsapi 导出 DnsQueryRaw、DnsCancelQueryRaw、DnsQueryRawResultFree，但 Runtime 未拦截 Raw。首版通过动态导出 Hook Raw，在 strict 中同步 ERROR_NOT_SUPPORTED；不伪造 Raw callback/result/cancel，也不新增旧 OS 静态导入。
5. DnsQueryEx 的明确 Host 与空 request/result 分开；strict 空输入本地 ERROR_INVALID_PARAMETER。空输入有 native call 的源码证据，但没有它产生真实宿主网络查询的证据。

已有 Query Engine 继续承担所有受支持请求。绝对 deadline 在异步入队前冻结，A/AAAA、CNAME、各配置上游及 UDP→TCP 共用总预算，取消后不尝试下一项。明确尚未支持的输入本地失败，不代表它已具备解析功能。

## 真正 ExW async 的后续契约

接收时复制 name/service/hints，取 caller timeout 与 Profile 总预算的较短 deadline。事件模式要求 manual-reset hEvent；callback 模式 hEvent=NULL。结果与状态先发布，再通知；借用 result slot、OVERLAPPED、上下文只使用到通知。generation token/pending map 防止 stale cancel、复制 token、重入与复用 storage 错误；成功/失败/timeout/cancel 单一完成者。取消结果 WSA_E_CANCELLED，invalid/stale WSA_INVALID_HANDLE。GetAddrInfoExOverlappedResult 必须有 native 对照，不猜测 Internal 布局。

微软 ExA 参数约定仍把 timeout/overlapped/completion/handle 作为保留 NULL，不能未经真实 ABI 验证便声称支持 ANSI async。

## 必要实际验证

ExA/W async、非 DNS namespace/provider GUID 同步错误且无 pending/回调/event/token；DnsQueryEx 空输入及错误 version/options/interface/server list；Raw name/packet 实际同步拒绝和 Host 对照；必需 attach 或完整 snapshot 故障入口未运行；保留全 QTYPE/native free/CNAME/root/unknown/UDP→TCP/TCP_ONLY/来源及 Question 验证。

Profile fixture 收到请求或源码无 native call，不能证明完整零 Host DNS。专用宿主 DNS sink、系统网络观察或等效证据未执行时保留零流量未证明。应用自带 DNS transport 与未支持的网络路径仍依赖后续 WFP 层，不属于本 API 路由的强隔离承诺。

官方契约：[GetAddrInfoExW](https://learn.microsoft.com/en-us/windows/win32/api/ws2tcpip/nf-ws2tcpip-getaddrinfoexw)、[GetAddrInfoExCancel](https://learn.microsoft.com/en-us/windows/win32/api/ws2tcpip/nf-ws2tcpip-getaddrinfoexcancel)、[DnsQueryRaw](https://learn.microsoft.com/en-us/windows/win32/api/windns/nf-windns-dnsqueryraw)。
