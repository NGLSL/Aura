# DoH bootstrap WinHTTP 原型证据

Date: 2026-10-04
Related: [18: DoH 无宿主 DNS bootstrap 选型原型](../issues/18-ticket.md) · [DNS transports 规格](../../dns-transports/spec.md)

**结论：本机 x64/x86 的 bootstrap + URL 身份 + HTTP/2 功能验证通过；票 18 完整门禁为 No-Go，不能解除票 19 的阻塞。** 本轮没有证明宿主 DNS 或证书辅助网络请求为零，也没有目标 Windows 版本矩阵。No-Go 是当前证据不足，不能据此断言 WinHTTP 无法满足后续验收。

本原型由主 Agent 明确授权作为无 VM 时可独立进行的工作，不认领票 19，不把票 14 尚未完成的配置全链当成已交付。没有修改产品 Runtime/Core、已有 DNS fixture、宿主 DNS、代理、信任库或 hosts；没有安装证书、跳过证书校验、部署外部服务或手写 HTTP/2。

## 可复现入口

文件位于 [tools/envbox-doh-prototype](../../../tools/envbox-doh-prototype/)：`CMakeLists.txt`、`main.cpp`、`run.ps1`、`observe-clienthello.py`。

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-doh-prototype/run.ps1
Get-Content target/doh-prototype-results.json -Raw
Get-Content target/doh-clienthello.json -Raw
```

脚本编译 x64 与 Win32，随后使用 WMI `Win32_Process.Create` 启动隐藏的独立 host worker。它清除继承的 `ENVBOX_*` 并检查 worker 模块中没有 `envbox-runtime*`。原型 EXE 也拒绝在已载入 Aura Runtime 的进程中运行。这样避免本次 Codex 所在 Aura 进程树把测试 profile/连接策略覆盖成当前活动环境。

输出保存在 `target/`，不作为持久规格。公共服务可变化，必须读取实际结果与 callback flags，不把退出码非零一律算成证书负向通过。脚本不提权；Python observer 仅绑定 `127.0.0.1:18443`，接收两次 ClientHello 后退出。外部请求只针对显式 literal bootstrap IP：Cloudflare `1.1.1.1:443` 和证书负向夹具 `104.154.89.105:443`。

## 本机实际结果

OS：Windows `10.0.26200`；系统 `winhttp.dll` 文件版本 `10.0.26100.8875 (WinBuild.160101.0800)`。MSVC `19.44.35228.0`，SDK `10.0.26100.0`。两种架构都实际执行，不只完成编译；Win32 是本机 WOW64 执行，不代表独立 32 位 Windows 系统。

| 场景 | x64 | x86/WOW64 | 证据与范围 |
| --- | --- | --- | --- |
| `cloudflare-dns.com` URL 身份，bootstrap `1.1.1.1` | PASS，exit 0 | PASS，exit 0 | `WINHTTP_OPTION_CONNECTION_INFO` 返回 `remote_ip=1.1.1.1`；protocol-used=1，即 HTTP/2；HTTP 200；`application/dns-message`；61-byte DNS body，ID `0x1234`、QR=1 |
| `wrong.identity.invalid` URL 身份，相同 bootstrap | 正确拒绝，exit 5 | 正确拒绝，exit 5 | `certificate_failure_flags=0x00000010`（CN invalid）、request error `12175`；没有设置任何 IGNORE_CERT_* |
| `expired.badssl.com`，literal `104.154.89.105` | 正确拒绝，exit 5 | 正确拒绝，exit 5 | `certificate_failure_flags=0x00000020`（date invalid）、request error `12175`；没有把普通超时算为过期证明 |
| URL 身份 `cloudflare-dns.com`，本地 bootstrap `127.0.0.1:18443` | SNI 原文正确，ClientHello 463 bytes | SNI 原文正确，ClientHello 196 bytes | 本地被动 TLS fixture 实际读到 `cloudflare-dns.com`；两者 offered ALPN 包含 `h2` 和 `http/1.1`；fixture 故意中止握手，不能当作 TLS/HTTP 正向成功 |
| HTTP authority | 配置并查询到 `Host: cloudflare-dns.com` | 同左 | 原型从 URL 名称生成 Host，WinHTTP request-header 查询可见；没有远端 HTTP/2 服务端日志，不能把 WinHTTP 抽象的 `HTTP/1.1` raw-header 展示当成真实 HTTP/2 wire capture |

原始日志：`target/doh-{64,32}-positive.log`、`target/doh-{64,32}-wrong-identity.log`、`target/doh-{64,32}-expired.log`、`target/doh-{64,32}-clienthello.log`、`target/doh-clienthello.json`、`target/doh-prototype-results.json`。

过期负向使用 [badssl 官方证书测试站](https://github.com/chromium/badssl.com)。它不承诺长期固定行为；本次是否命中过期证书由原生 callback 的 date-invalid flag 判定，没有通过宿主 DNS 自动寻找它。

## 官方契约与实际策略

WinHTTP 的 [option flags 文档](https://learn.microsoft.com/en-us/windows/win32/winhttp/option-flags)说明 `WINHTTP_OPTION_RESOLUTION_HOSTNAME` 可在发送前覆盖解析名称；HTTP/2 的 enable/protocol-required/protocol-used 可分别设置能力、禁止其他协议和读取实际协议。本文只用该文档解释 API，不用它替代实际 IP、SNI 和协议观察。

原型保持 `WinHttpConnect(URL_HOST)`，随后在 request 设置 resolution-hostname 为 literal IP。HTTP authority 显式来自 URL_HOST。TLS 至少 1.2，HTTP/2-required，session 使用 `WINHTTP_ACCESS_TYPE_NO_PROXY`；未调用 auto-proxy/PAC API。request 禁 cookies、自动 authentication 和 redirects，并设置 redirect-never、autologon-high。所需 option setter 任一失败，在请求发送前拒绝；不切宿主 bootstrap、不添加替代 DNS、不用忽略证书错误重试。

Microsoft 的 [WINHTTP_CONNECTION_INFO 契约](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/ns-winhttp-winhttp_connection_info)定义的是产生响应的连接源/目标地址。本次正向成功后查询该结构，作为远端 IP 的 API 观察。负向日志的 connected-to-server 回调是连接进度观察，不声称是成功 HTTP response 的 connection-info 结果。

Microsoft 的 [status callback 文档](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpsetstatuscallback)提供解析、连接和安全失败的进度通知。实测这些解析通知的值是 literal IP；它们没有为整个 Windows DNS Client、Cryptnet、OCSP/CRL/根信任自动更新提供完整抓包或流量覆盖。

## 证书辅助网络策略与观测缺口

原型必需设置 `WINHTTP_OPTION_SERVER_CERT_CHAIN_BUILD_CACHE_ONLY`（SDK option 199）为 TRUE；x64/x86 本机 setter 均成功。没有设置 `WINHTTP_OPTION_DISABLE_CERT_CHAIN_BUILDING` 或任何 `SECURITY_FLAG_IGNORE_*`，也没有启用 SSL revocation 或忽略离线吊销错误。

SDK 中的 option 148 与 cache-only、disable-AIA、revocation-cache-only 常量可在 [Microsoft 发布的 WinHTTP header](https://raw.githubusercontent.com/microsoft/win32metadata/main/generation/WinSDK/RecompiledIdlHeaders/um/winhttp.h)核查。最初实验对 148 写入：request 返回 `12019`（incorrect handle state），session 返回 `12018`（incorrect handle type）；最终不保留这两个错误 setter，而尝试 request query，返回 `87`。因此 **不能声称 148 接受了 AIA/CRL 关闭策略，也不能把 option 199 的名称或 setter 成功解释为整个证书验证路径零网络的证明**。

本轮没有关闭证书验证，也没有导入根证书。链缓存为空、冷启动缺中间证书、根信任自动更新、OCSP/CRL 等辅助路径没有专用可观测夹具；未取得系统范围 DNS/网络抓包。原型明确输出 `zero_host_dns=UNPROVEN_CALLBACKS_ARE_NOT_PACKET_CAPTURE`，总结果保持 No-Go。

## 门禁未通过项与下一步

票 18 可以记录以上进展，但不能完成：

- 未建立已知失败 Host DNS/PAC/代理/证书辅助网络路径的完整观测，不能计为零请求。
- 远端 HTTP/2 服务端 authority 与 TLS SNI 尚无同一专用受控服务端的完整记录；当前 SNI 和协议证据来自两个独立场景。
- 未覆盖声明支持的 Windows 版本/补丁矩阵、冷信任材料、信任链负向与吊销策略。过期/错误身份已有本机两架构观察，不能替代这些门槛。
- 未进入正式 DoH router、typed upstream、strict lifecycle、总 deadline/cancel/fallback/连接池实现或与 product Runtime 的链接验收。

优先选项是在具备隔离测试目标后继续验证现有 WinHTTP 原型，增加专用受信任夹具、Host DNS/PAC sink、证书辅助地址 sink 和系统流量关联；成本主要是测试环境与观测，而本轮未发现 bootstrap/SNI/HTTP2 功能阻碍。保留 WinHTTP 可避免新增成熟 HTTP/TLS 栈的打包维护成本。

若 cache-only/辅助网络策略无法被明确控制和证明，再评估成熟库，不能临时手写 HTTP/2：

| 候选 | 官方能力依据 | Aura 成本与待证事项 |
| --- | --- | --- |
| libcurl + HTTP/2 + 明确信任材料的 TLS backend | [CURLOPT_CONNECT_TO](https://curl.se/libcurl/c/CURLOPT_CONNECT_TO.html)明确说明连接地址不改变 SNI、证书校验或应用协议身份；[CURLOPT_RESOLVE](https://curl.se/libcurl/c/CURLOPT_RESOLVE.html)支持指定地址 | C++ API 接入容易；需要选定并锁定 HTTP/2/TLS backend，维护 x86/x64 构建、依赖版本、安全更新、许可证及 CA/吊销材料；不能默认系统 curl.exe 的构建能力适合 DLL 内部使用 |
| reqwest/hyper + rustls 的进程内静态 FFI 模块 | [reqwest ClientBuilder](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)提供显式 resolve/no_proxy/redirect/TLS 配置；[rustls 文档](https://docs.rs/rustls/latest/rustls/)描述其 TLS 与认证能力 | 可复用现有 Rust 工具链，但 C++ Runtime 需要新的 FFI、两架构 staticlib、异步执行/取消/资源生命周期；CA 与吊销策略须明确并实际验证，不能从 Rust 库名称推定零 Host DNS |

这些是后续选型成本分析，不是已构建或已验收的替代实现；没有擅自引入依赖或解除票 19 的门禁。
