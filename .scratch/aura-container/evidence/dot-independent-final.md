# DoT 独立验收与剩余边界

Date: 2026-10-06
Status: independent-native-and-injected-validation
Scope: ticket 17, no VM, no certificate-store or host-DNS mutation

这份记录只覆盖当前分支可以在宿主机完成的 DoT 证据。它不把本地证书夹具、一次网络连接或 API 计数扩大解释成完整的系统级无泄漏保证；驱动/WFP、系统级抓包、其他 Windows 版本和重启故障仍属于后续验收。

## 当前实现边界

`runtime/src/dns_dot.cpp` 使用显式 literal IP 建立非阻塞 TCP 连接，TLS 由 Schannel 完成，最低 TLS 版本为 1.2；`server_name` 作为 TLS 身份输入，literal IP 另外要求证书的 IP SAN。DNS-over-TLS 使用两字节长度前缀，单次调用创建并关闭 socket 和 Schannel context，不使用连接池，也不调用名称解析或 URL 获取。

证书链使用本机 Windows 信任材料，并设置以下离线策略：

- `CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL`
- `CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY`
- `CERT_CHAIN_DISABLE_AIA`
- `CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE`

链验证、TLS policy 和身份验证任一失败都结束当前上游尝试；不会把 DoT 失败转为宿主 DNS。缺少本地吊销材料返回 `RevocationUnavailable`，撤销证书返回 `Revoked`，身份不符返回 `Identity`。

## 两架构本地传输夹具

既有独立夹具 `tools/envbox-dns-dot-fixture/run.py` 对同一份 production `dns_dot.cpp` 编译 x64 和 x86，并使用独立内存 root/CRL；没有安装证书、修改宿主 DNS 或改变网络策略。固定产物 `target/dot-transport17-green.log` 的结果为两架构各 16 个场景，共 32/32：

- 分片 TLS/DNS framing、IP SAN、最大 65535 字节响应成功；
- 16 次顺序连接后句柄计数保持不变：x64 `214 -> 214`，x86 `233 -> 233`；
- 不可信、过期、名称不符、撤销证书和无本地 CRL 分别失败，错误类别分别为 certificate(6)、identity(9)、revoked(8)、revocation-unavailable(7)，且证书失败时没有发送 DNS application data；
- 截断、多响应、非法/超限响应失败；
- 握手/读取 deadline 返回 `Deadline(3)`，握手/读取 cancel 返回 `Cancelled(2)`；网络等待每次最多轮询 50ms。

这证明了 transport 的 framing、错误分类和正常资源回收。CryptoAPI 的证书链构建是同步调用，只能在调用前后检查 deadline/cancel，Windows 没有可用于中断该调用的安全契约；因此“链构建中途取消”仍是未验证限制。

## 当前 Windows 信任库下的真实公共 DoT

使用与产品相同的 `DnsDotExchange`（没有 `ENVBOX_DNS_TRANSPORT_TESTING` chain-engine 覆盖）编译了新的 x64/x86 fixture：

```text
x64 exe: DAE8228BB94DD9A6953F27EBEBA891F9A1F3DB84C5321EB2C8E10D5A4E8E5A97
x86 exe: FD809D0C503D735881D123E12755DD23A39CF29F33778FA5EBA852AAD1BB03CA
```

针对显式 `1.1.1.1:853`、TLS identity `cloudflare-dns.com` 的独立连接，当前宿主机两架构均成功返回 102 字节 DNS packet，`error=0`，进程退出码为 0：

```text
x64: result=102 error=0 elapsed=922
x86: result=102 error=0 elapsed=922
```

`1.0.0.1:853` 与同一 identity 的 x64/x86 连接也成功。错误身份 `wrong.invalid` 在两架构均返回 `result=0 error=9`，未发送 DNS application data。`9.9.9.9:853` / `dns.quad9.net` 返回 `RevocationUnavailable(7)`，这是严格本地缓存吊销策略的拒绝结果，不能通过关闭校验来“修复”。首次 Cloudflare 尝试在本机缓存尚未准备好时也曾返回 error 7，随后当前系统已有材料后稳定成功；这说明公共正向证据依赖宿主当前信任/吊销缓存，不能替代跨机器资格。

## 真实 Runtime 注入

新增 `tools/envbox-dns-dot-fixture/run_public_injected.py`，使用真实 CLI、Probe 和 Runtime DLL，Profile 只配置一个显式 DoT 上游 `1.1.1.1:853 / cloudflare-dns.com`，`virtual_view + strict=true`，没有 UDP/TCP/Host fallback。脚本每次只运行一次，不自动重试；每次结果写入唯一的 `target/dot-public-injected-evidence-<uuid>.json`。该 JSON 保存清理的 `ENVBOX_*` 环境键、CLI/Probe/DLL 路径及 SHA-256、fresh Host Probe 原文、每个 Probe 的完整 stdout/stderr，以及 live Probe 中实际加载的 Runtime 模块路径和 hash。

本轮使用 root 冻结的 `target/nonvm-final-runtime-v2` pair 做了各一次 fresh Host + public injection。两次都使用 x64 CLI；x64 使用 `target/debug/envbox-probe.exe`，x86 使用 `target/x86-probe-only/i686-pc-windows-msvc/debug/envbox-probe.exe`，避免把 x86 CLI 的启动问题混入 DoT 结果：

- x64 单次通过：A/W/UTF8/Ex/async 均 `DnsRR_Status=0`、`DnsRR_Records=1`，见 `target/dot-public-injected-v2-x64-single.log` 和 `target/dot-public-injected-evidence-2b2a6cf2567b47b6b874547c9b286b7c.json`；
- x86 单次通过：A/W/UTF8/Ex/async 均 `DnsRR_Status=0`、`DnsRR_Records=1`，见 `target/dot-public-injected-v2-x86-single.log` 和 `target/dot-public-injected-evidence-746ea8ca4ba14aa785591039983aa6dc.json`；
- x64 Runtime SHA-256 为 `CA8283ADAE000DBEAAE65902A10F2E0E05B94C38C14F7B3652EDD3066C43D965`，x86 Runtime SHA-256 为 `5D2FD1038748F0D579F5B2EB59EBAEECDFF6C7846D12FD93876875696B6CEBE7`；
- 两份 evidence 都是 `attempt=1`、`retry=false`。fresh Host Probe 与 Python controller 前后均记录 `Runtime modules=[]`、Host Probe exit 0；live Probe 实际加载的路径分别为 `C:\Users\admin\AppData\Local\com.aura.envbox\runtime\bundle-2-000000000007f600-49f0b8443556f042\envbox-runtime64.dll` 和 `envbox-runtime32.dll`，实际模块 hash 分别与上述 v2 DLL 完全一致；
- evidence 同时保存了 CLI、Probe、DLL 的实际路径和 SHA-256，以及每个 API 的完整 Probe stdout/stderr，临时 config root 均记录 `cleanup.removed=true`、无 cleanup error。

此前 V3 pair 的 `target/dot-public-injected-script32-v2.log`、`target/dot-public-injected32-retry.log` 等 timeout/retry 日志仍保留为历史样本，不混入本次 v2 结果，也不作为当前双架构结论。本次 v2 是本轮最后冻结的 Runtime pair；若后续修改 Runtime 源码并产生新 pair，需要为新产物重新取得独立回归证据。

当前证据没有做系统范围抓包，也没有声称所有进程/服务的 DoT、DoH 或直接 AFD 流量都被约束。

## 辅助流量、取消和资源证据

静态源码和配置证明 DoT 连接使用 literal IP，未调用 `getaddrinfo`/`DnsQuery*`/WinHTTP URL 获取；CryptoAPI 使用 cache-only/AIA-disabled 参数，故该路径不会主动通过链获取 CRL。但本轮没有完成覆盖全系统的 ETW/WFP/抓包，也没有把 DoH fixture 的 API trap 结果冒充 DoT 的全局观测。

本地 fixture 已覆盖网络等待期间的 cancel/timeout 和 16 次连接的句柄边界。未覆盖的部分是 CryptoAPI `CertGetCertificateChain` 执行期间的可中断性、每种 Windows 版本的系统信任缓存、以及公共端点辅助请求的全局观测；实现保持严格失败，不安装信任材料、不启用 URL retrieval、不改变宿主网络。

## 结论

票 17 的独立 transport、TLS identity、严格失败、framing 和资源边界已有宿主证据；本轮还证明了选定 v2 Runtime pair 在 x64/x86 真实 Profile 路径上的一次完整正向运行，并记录了 fresh Host 与 live module/hash provenance。整票仍不能关闭：这只是每架构一次的 IPv4 公共端点 smoke，系统级辅助流量捕获、其他 OS 矩阵、长时间/重复公共网络矩阵、中途 CryptoAPI cancel，以及任何后续 async Runtime freeze 的最终回归仍未完成。这些是验收边界，不是通过放宽证书或 Host fallback 来规避的理由。
