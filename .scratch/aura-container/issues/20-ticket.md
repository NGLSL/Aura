# 20: 四种 DNS 传输实际注入验收

Stage: P3
Status: ready-for-agent
Blocked by: [16: 严格解析所有入口与异步取消](16-ticket.md)、[17: DoT 传输及 TLS 校验](17-ticket.md)、[19: DoH 正式 transport 实现](19-ticket.md)、[11: GUI 重连与幂等 Stop/Stop all](11-ticket.md)
Parent: [总规格](../spec.md) · [完整实施规划](../implementation-plan.md)

**What to build:** 确认 UDP/TCP/DoT/DoH、任意 QTYPE、严格失败和父子解析在真实 Runtime 中工作，关闭 GUI 后保持相同行为。

## 负责模块与契约

Probe、DNS/Host 本地 fixtures与 GUI/CLI能力呈现。检验已实现路径，不用测试通过替代应用自带 DNS/WFP 保证。

## 不包括

不扩大网络隔离、不发布或安装；本票不从单 transport 成功推断全部 API成功。

## 验收标准

- [ ] 四种 transport 分别执行 A/AAAA/HTTPS/SVCB/TXT/PTR/SRV/CNAME/NS/root/未知类型，native free 与返回值正确。
- [ ] 混合顺序、仅加密、证书失败、全部失败、bootstrap错误、取消和 deadline均与配置一致，无隐藏明文/Host fallback。
- [ ] strict 的所有声明 resolver 入口在负向测试中 Host fixture 零请求；Host控制保持正常且宿主全局 DNS 不变。
- [ ] 实际 x64/x86 父子和关闭 GUI 后解析持续使用同一快照；重连看到真实 transport 能力。
- [ ] 并发/重复取消/连接复用长批次资源稳定；本地夹具与真实服务证据分别标记，不宣称浏览器自带 DoH 已阻断。

## 验证证据

归档模块路径/hash、快照、系统/架构、DNS wire及Host负向记录、GUI退出后结果和资源测量；遗漏必需项保持未完成。

## 关联验收

A04、A14、A16、F03、F10。DNS 相关细节遵循 [DNS transports 规格](../../dns-transports/spec.md)。本票完成仅代表该切片；总目标只有全部最终验收通过才完成。依赖未完成时不得认领执行，ready-for-agent 只表示票内容已可执行。
