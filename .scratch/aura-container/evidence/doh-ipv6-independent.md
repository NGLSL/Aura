# DoH IPv6 与 IP 身份本机验收

Date: 2026-10-06
Baseline: `a5baed8`
Related: [18](../issues/18-ticket.md)

## 已完成

扩展现有真实 TLS/HTTP fixture：新增 IPv4 IP URL 正向，证书含对应 IP SAN，HTTP authority 保持 IP 与实际端口，TLS 不发送 DNS SNI。两架构分别成功，禁止 DNS/getaddr、WinHTTP/PAC、CryptoAPI chain API、非配置 endpoint 和 canary 计数均为 0。

最终 fresh WMI Host PID 13076、Runtime modules=0，`target/doh-nonvm-ipv6-qualified.log` 共 **58 个已执行场景通过**（每架构 29 个），exit 0。该数量不包含未执行的 IPv6 场景。Rust fixture 在独立 feature-enabled target 重新构建，不改变默认 product backend。

## IPv6 环境限制

保留五个 IPv6 场景/架构：hostname URL 的 h2/h1、bracketed IP URL 与 IP SAN、错误 IP 身份、读取取消。预检使用普通 TCP socket 连接自己在 `::1` 上的 listener，不使用 DoH backend 或 instrumentation；失败则打印 `ipv6_preflight=unverified` 与原始系统错误，跳过场景不计通过。

首次 fresh WMI PID 22752 的 DoH `::1` 连接返回 Network 4，listener 没有连接，trap 允许一次且未拒绝 endpoint。随后 fresh WMI 独立 Python TCP 对照 PID 17780 同样返回 **WinError 10013**；`target/ipv6-local-control.log` 保存地址与错误。最终预检也返回 10013。本机接口存在不能证明允许 IPv6 socket 连接；具体拒绝策略来源未归因，不能猜测为 Aura 或某个网络软件。

用户随后明确说明当前已关闭 IPv6。该环境配置与本机连接失败一致；按环境未启用记录未验证，不尝试启用 IPv6 或修改网络策略。此前具体策略来源未归因的诊断日志保持原样。

首次矩阵未完成，其日志 `target/doh-nonvm-ipv6.log` 保留，不计为通过。最终矩阵明确写 `IPv6=UNVERIFIED`。没有改防火墙、网络接口或宿主安全策略。IPv6 数据面仍需允许 IPv6 的测试环境验证，不能解除票 18 总体门槛。
