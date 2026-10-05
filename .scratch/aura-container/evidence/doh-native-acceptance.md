# DoH 原生生产信任正向验收

Date: 2026-10-06
Related: [18 选型门禁](../issues/18-ticket.md) · [DoH 原型证据](doh-rustls-prototype.md) · [DoH 设计](doh-rustls-design.md)

## 结论

票 18 的原生公共 DoH 正向门槛在本机仍未通过，不能解除票 19，也不能启用产品 DoH。

这次使用的是 `envbox-dns-doh` 的产品默认 staticlib（`--no-default-features`），由 MSVC x64 和 Win32 DLL 真实链接，再由 fresh WMI worker 加载调用。没有 fixture trust、没有安装证书、没有修改宿主 DNS/代理/信任库，也没有提权。默认 Windows ROOT/CA/Disallowed 与本地 CRL/cache 快照加载成功，但两个公共 IPv4 DoH 端点都在 TLS 证书阶段返回 `EnvBoxDohCertificate`（error 6）；同源 Rustls diagnostic 已将它定位为 `InvalidCertificate(UnknownIssuer)`，没有 HTTP/DNS response。两个公共 IPv6 端点均为 `EnvBoxDohNetwork`（error 4），本机没有可用 IPv6 route。

因此，本机能证明的是：literal bootstrap 的调用路径确实进入了默认离线信任 loader，并返回了严格的证书失败；本机不能证明公共 DoH 正向成功，也不能把 IPv6/全机流量观测门槛记为通过。

## 可重跑入口

独立 probe 位于 [`tools/envbox-dns-doh-native-probe`](../../../tools/envbox-dns-doh-native-probe/)。它不是 workspace Cargo package，不启用 `fixture-trust`，不会改变产品 gate。

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/run.ps1
Get-Content target/doh-native-acceptance-results.json -Raw
Get-Content target/doh-native-acceptance-64.log -Raw
Get-Content target/doh-native-acceptance-32.log -Raw
```

每次运行先生成唯一 `run_id`，WMI worker 将结果写入带该 ID 的临时结果文件；父脚本最多等待三分钟，并只接受 `run_id` 相同且 `completed` 字段存在的 JSON。完成后才复制稳定结果/日志路径，因此不会把旧结果当成当前运行。当前默认 positive gate 仍是 No-Go：即使 harness 完成，脚本也会以非零退出并保留 `gate=false`、`wire_positive` 和每个 typed error；诊断脚本则允许 typed 失败以零退出，但 JSON 明确标记 `gate=false`。

三个 wrapper 的 `-RunId` 仅供 WMI worker 内部交接：顶层入口拒绝外部传入并自行生成新的 N 格式 GUID，worker 仅接受 32 位十六进制 GUID；创建 worker 前若对应结果文件已存在则拒绝复用。

为取得稳定 C ABI 没有暴露的 Rustls 内层错误，另有一个隔离 diagnostic 直接复用生产 `Snapshot::load()`、`Snapshot::verifier()` 和同一自定义 verifier，只做 literal TCP/TLS handshake，不发送 HTTP，也不使用 accept-all verifier：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/diagnostic/run.ps1
Get-Content target/doh-native-diagnostic-results.json -Raw
Get-Content target/doh-native-diagnostic-64.log -Raw
Get-Content target/doh-native-diagnostic-32.log -Raw
```

Peer-chain capture (Schannel，记录后主动拒绝握手，不作为产品信任结果)：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/capture-chain.ps1
Get-Content target/doh-native-chain-capture.log -Raw
```

capture 同样使用唯一 `run_id`、bounded WMI wait，以及每 endpoint 15 秒 TCP/TLS deadline 和 60 秒 worker 总预算。证书回调由独立 C# delegate 执行，避免 PowerShell scriptblock 在 TLS thread-pool 线程上没有 runspace；它记录 Schannel chain 后始终返回拒绝。若未来出现 `handshake=accepted_unexpectedly`，worker 会设置 `invalid_handshake=true`、`worker_exit=1`，父脚本随之非零退出；本次实际 capture 最新成功运行 `run_id=ff166d53d2f24b61a18b7c95b1e70614` 完成且两个 endpoint 都记录了 peer chain 和预期握手失败。

脚本先在隔离 target 目录编译 x64 与 i686 产品 staticlib，检查 staticlib 中不存在 `query_fixture`、`Snapshot::fixture` 等测试信任符号，再构建原生 DLL/host。脚本使用 WMI `Win32_Process.Create` 启动隐藏 worker；worker 清除继承的 `ENVBOX_*`，检查自身没有 `envbox-runtime*` 模块，然后分别执行 x64 与 x86 host。

2026-10-06 的工具回归运行也验证了这套异步边界：默认 diagnostic 最新 `run_id=68370183724149818ba8afd4fa73cbeb`（此前同样验证过 `0a610477138d44c7a89a07f579661f14`）和 AuthRoot research variant `run_id=40086e28f26a49aaa47d6c0d7729a85e` 均由 fresh WMI worker 完成，两架构子进程 `exit=0`，结果分别保留 `UnknownIssuer` 与 `UnknownRevocationStatus`，JSON 的 `gate=false`。默认 acceptance 最新 `run_id=0c6932dddc7749ff854e50f0a3a143f7`（此前也完成过 `40d6af88b013430ca7d1e698e2c93010`）的 worker `PID=11356` 完成，两架构 host `exit=0`，父脚本按预期以非零退出，因为 JSON 为 `completed=true`、`harness_completed=true`、`worker_exit=0`、`wire_positive=false`、`gate=false`；其每次结果和日志均带 run id。这样 harness completion 和 positive acceptance 不再混为一谈。

## 实际环境与产物

| 项目 | 实测结果 |
| --- | --- |
| Windows | Windows 11 教育版，10.0.26200，x64 |
| Rust | `cargo 1.99.0 (5f94df478 2026-08-27)` |
| fresh WMI worker | latest acceptance PID `11356`、default diagnostic PID `7948`、AuthRoot research diagnostic PID `18852`，均 `runtime_modules=0` |
| x64 probe | latest acceptance PID `29376`，`runtime_modules=0`，status `0` |
| x86/WOW64 probe | latest acceptance PID `29372`，`runtime_modules=0`，status `0` |
| URL | `https://cloudflare-dns.com/dns-query`、`https://dns.google/dns-query` |
| DNS query | `example.com A IN`，query ID `0xa042` |
| IPv6 route | `ipv6_route=no` |

`status=0` 只表示 native host 调用完成；每个 endpoint 的 typed error 才是正向/失败判定依据。外层 `run.ps1` 还会检查当前唯一 run 的 JSON gate，因此本次即使两个 host 都是 `exit=0`，脚本仍以非零退出并保留结果。原始结果在 `target/doh-native-acceptance-results.json` 及两个架构日志中，带 run id 的文件是本次证据的优先来源。

产品源码用于构建的关键 hash：

| 文件 | SHA-256 |
| --- | --- |
| `crates/envbox-dns-doh/src/transport.rs` | `CB09831DA7B664ECFD78F89496A87858BA53D1C1802A1C320C4747CDC149DB47` |
| `crates/envbox-dns-doh/src/trust.rs` | `497C220BE535AA11075D4CE9B7B56609D595D431663D54B445319609E515CF7D` |

原生 DLL、host、staticlib 与 PE/import 检查保留在 `target/doh-native-acceptance64/` 和 `target/doh-native-acceptance32/`。这些构建产物属于本地证据，不应当作为发布包使用。

## Endpoint 结果

同一份 probe 在 x64 与 x86/WOW64 得到相同结果：

| 架构 | URL / literal bootstrap | typed result | response | 解释 |
| --- | --- | --- | --- | --- |
| x64、x86 | `cloudflare-dns.com` / `1.1.1.1` | `error=6 certificate` | `length=0` | TCP/TLS 路径进入证书验证，但没有可交付的 HTTP/DNS response |
| x64、x86 | `cloudflare-dns.com` / `2606:4700:4700::1111` | `error=4 network` | `length=0` | 本机没有 IPv6 route；没有把该结果算成 TLS 或 DoH 正向 |
| x64、x86 | `dns.google` / `8.8.8.8` | `error=6 certificate` | `length=0` | TCP/TLS 路径进入证书验证，但没有可交付的 HTTP/DNS response |
| x64、x86 | `dns.google` / `2001:4860:4860::8888` | `error=4 network` | `length=0` | 本机没有 IPv6 route；没有把该结果算成 TLS 或 DoH 正向 |

稳定 C ABI 将内层 `UnknownIssuer` 归类为 `certificate`，没有在 ABI 中泄漏 Rustls 具体变体。隔离 diagnostic 直接复用生产 verifier，在 x64/x86 两架构都得到：

```text
case=cloudflare_ipv4 ... result=tls_error=Custom { kind: InvalidData, error: InvalidCertificate(UnknownIssuer) } rustls_inner=InvalidCertificate(UnknownIssuer)
case=google_ipv4 ... result=tls_error=Custom { kind: InvalidData, error: InvalidCertificate(UnknownIssuer) } rustls_inner=InvalidCertificate(UnknownIssuer)
```

它没有绕过证书验证，也没有发送 HTTP。独立 Windows chain 诊断为 Cloudflare 当前链：

- leaf `F88635017260D40B9EB417BEE73737911B630E59`，`cloudflare-dns.com`；
- intermediate `9219612E901C4904F9835EE8C2ADD54FF0B797FD`，`SSL.com SSL Intermediate CA ECC R2`；
- root `C3197C3924E654AF1BC4AB20957AE2C30E13026A`，`SSL.com Root Certification Authority ECC`。

同样的直连 capture 对 Google 得到的是另一条公开链，并不与 Cloudflare 共用 root：

- leaf `9ED64D43005C45EC8CD03BBB8C4F846073C58636`，`CN=dns.google`，issuer `CN=WE2, O=Google Trust Services, C=US`；
- intermediate `4D9ACB313D73DA2E9EB451A7CF6309AF45C993C1`，`WE2`，issuer `GTS Root R4`；
- cross/root candidate `932BED339AA69212C89375B79304B475490B89A0`，`GTS Root R4`，issuer `GlobalSign Root CA`；
- self-signed root `B1BC968BD4F49D622AA89A81F2150152A41D829C`，`GlobalSign Root CA`。

同源 Rustls diagnostic 的 recording verifier 在调用生产 verifier 前记录了 TLS peer 实际交给 verifier 的链。默认 native run 的 x64/x86 记录相同；它没有改验证结果，也没有 accept-all：

```text
peer_chain_server_name=DnsName("cloudflare-dns.com") intermediate_count=1
peer_chain=leaf subject=cloudflare-dns.com issuer=SSL.com SSL Intermediate CA ECC R2 der_sha256=E3B02826789D653D224D3EDACBE4E877CB7286FC4C922672F6226741CA57AD65
peer_chain=intermediate[0] subject=SSL.com SSL Intermediate CA ECC R2 issuer=SSL.com Root Certification Authority ECC der_sha256=948B7111AF42F546D579CFF5CE2BDEC82134DD9914842BDDB0C52872EB604E39
peer_chain_server_name=DnsName("dns.google") intermediate_count=2
peer_chain=leaf subject=dns.google issuer=WE2 der_sha256=BD5A8A7F39A4A78E048D29AB4F0152CB3652CF9BE183D54CFEE5DD5B228701AB
peer_chain=intermediate[0] subject=WE2 issuer=GTS Root R4 der_sha256=9C3F2FD11C57D7C649AD5A0932C0F0D29756F6A0A1C74C43E1E89A62D64CD320
peer_chain=intermediate[1] subject=GTS Root R4 issuer=GlobalSign Root CA der_sha256=76B27B80A58027DC3CF1DA68DAC17010ED93997D0B603E2FADBE85012493B5A7
```

公共 Google endpoint 也在不同连接中返回过另一条合法链：AuthRoot 研究 variant 的 x64 记录 `dns.google → WR2 → GTS Root R1 → GlobalSign Root CA`，leaf/intermediate DER SHA-256 分别为 `17DF23AACFEE217A0D0491BA34FAAB20ACBEF9A7AA9A4ACB544CC9F6ABCC98AE`、`E6FE22BF45E4F0D3B85C59E02C0F495418E1EB8D3210F788D48CD5E1CB547CD4`、`3EE0278DF71FA3C125C4CD487F01D774694E6FC57E0CD94C24EFD769133918E5`；同一 variant 的 x86 和默认 run 观察到的是上面的 `WE2/R4` 链。两条链都在 AuthRoot variant 下推进到 `UnknownRevocationStatus`。这说明公共 CDN/服务端可能按连接选择链，当前证据不把差异归因于中间设备；它只记录本机 literal destination 实际观察到的 peer 链。

Schannel 的独立 capture 使用同样的 literal IP，只记录 Windows 构造出的辅助 chain，随后由 validation callback 返回 false 使握手故意失败；它没有把 Schannel 的结果当作产品 Rustls 通过，也没有安装或接受新信任。Windows 将 Cloudflare 的 issuer 链到 SSL.com root `C3197C3924E654AF1BC4AB20957AE2C30E13026A`，将 Google 的 cross-signed GTS 链到 GlobalSign root `B1BC968BD4F49D622AA89A81F2150152A41D829C`。这两个 root 都不在生产使用的 LocalMachine physical `ROOT` registry store（只在本机 AuthRoot/logical collection 中可见），所以两个 `UnknownIssuer` 不能归因于“两个服务恰好共享一个缺失 root”。Schannel chain capture 也不等价于全机抓包，不能单独排除中间设备或代理改链；但同源 Rustls raw peer chain 的 leaf/intermediate subject/issuer 与两个公共端点预期主机名一致，且 probe 使用了 literal TCP destination。

Windows 的独立 `X509Chain` 检查显示：`NoCheck=True`，`Offline=False` 且状态为 `RevocationStatusUnknown, OfflineRevocation`，`Online=True`。这说明 Windows logical chain 可以找到这条链，但生产 loader 的物理 registry ROOT 集合没有提供其 trust anchor。它是辅助定位证据，不能把 `UnknownIssuer` 擅自改写为 `RevocationUnknown`，也不能通过在线获取 CRL 或放宽 revocation 来解除门禁。

诊断直接枚举生产使用的两个 `CERT_STORE_PROV_SYSTEM_REGISTRY_W` ROOT 物理 store：CurrentUser `entries=0`，LocalMachine `entries=22`，均找不到 Cloudflare root DER SHA-256 `3417BB06CC6007DA1B961C920B8AB4CE3FAD820E4AA30B9ACBC4A74EBDCEBC65`。同一证书存在于只读的 `HKLM\SOFTWARE\Microsoft\SystemCertificates\AuthRoot\Certificates\C3197C3924E654AF1BC4AB20957AE2C30E13026A`；Google 链的 GlobalSign root `B1BC968BD4F49D622AA89A81F2150152A41D829C` 也只在 AuthRoot/logical collection 可见，两个 root 的 `HKLM\...\Root\Certificates\<thumbprint>` physical entries 都不存在。本机 logical `Cert:\LocalMachine\Root` 有 75 个条目；生产 snapshot 最终只有 10 个 accepted ROOT。也就是说，当前 `UnknownIssuer` 的直接原因是：生产刻意读取的物理 `ROOT` 不包含 Windows AuthRoot provider 中的这两个公共链 anchor，而不是 Aura Runtime 注入或网络 resolver。

Microsoft 的 [CertOpenStore 文档](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certopenstore)明确区分 `CERT_STORE_PROV_SYSTEM_REGISTRY_W` 的单一 physical registry store 与 `CERT_STORE_PROV_SYSTEM_W` 的 logical collection；[system store locations](https://learn.microsoft.com/en-us/windows/win32/seccrypto/system-store-locations)列出 LocalMachine Root 的 `.Default.AuthRoot` sibling；[Windows certificate trust 文档](https://learn.microsoft.com/en-us/windows-server/identity/ad-cs/certificate-trust)说明 AuthRoot 由 Microsoft CTL updater 管理，且 trusted CTL 位于 `AuthRoot\AutoUpdate\EncodedCtl`。因此不能简单把 logical `Root` 全量导入，也不能把 AuthRoot 证书直接提升为 trust anchor 而跳过其 CTL usage、policy、NotBefore/disable 和 disallowed 语义。

### AuthRoot 只读实验的边界

为了区分“缺少 anchor”和“离线撤销材料不足”，diagnostic 提供了一个只在独立研究 target 中生效的环境开关：

```powershell
$env:DOH_DIAGNOSTIC_INCLUDE_AUTHROOT = '1'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/envbox-dns-doh-native-probe/diagnostic/run.ps1
Remove-Item Env:DOH_DIAGNOSTIC_INCLUDE_AUTHROOT
Get-Content target/doh-native-diagnostic-authroot-64.log -Raw
Get-Content target/doh-native-diagnostic-authroot-32.log -Raw
```

这个开关只让 diagnostic 的 `build.rs` 生成一个临时的 `trust.rs` 副本，把本机 `AuthRoot` physical store 的候选证书加入研究用 snapshot；它不修改 `crates/envbox-dns-doh`，不安装证书，不改变系统 store，也不改变产品构建。最新 x64 与 x86 fresh WMI 结果都把目标 root 纳入研究 snapshot（x64 日志的 `accepted_root_count=317`），Cloudflare 与 Google 的 IPv4 TLS 错误都从：

```text
InvalidCertificate(UnknownIssuer)
```

推进为：

```text
InvalidCertificate(UnknownRevocationStatus)
```

这项结果确认了两个 endpoint 各自的第一阻塞：Cloudflare 链的 SSL.com ECC root、Google 当前观测链的 GlobalSign Root CA（另一条连接观察到的 WR2/R1 链同样以 GlobalSign Root CA 收束）都只在 AuthRoot/logical 侧可见，没有被默认物理 `ROOT` snapshot 选入；它们不是同一个缺失 root。与此同时，研究 variant 也证明把 AuthRoot 证书直接并入 anchor 集合仍然不能通过严格离线 revocation。它不能被解释为“把 AuthRoot 全量加入就安全”，更不能用 `NoCheck`、在线 CRL fallback 或 accept-all verifier 绕过第二个失败。

只读 `certutil -verifyctl AuthRoot` 返回成功；本机 AuthRoot CTL 的 `SequenceNumber=1401dd3462c779aec7`、`ThisUpdate=2026/8/25 15:24`，并包含目标 root 的 SHA-1 subject identifier `c3197c3924e654af1bc4ab20957ae2c30e13026a`。目标证书的 `CERT_AUTH_ROOT_SHA256_HASH` property 存在，现有 `eligible()` 判断为 `Ok(true)`；但单证书上没有 `CERT_ROOT_PROGRAM_CERT_POLICIES`、name constraints 或 chain policies property。AuthRoot 的 program usage/policy 语义属于 CTL/系统 trust 管理，不能因为单证书的 `eligible()` 返回 true 就省略 CTL 解析和 freshness/disable 检查。

若后续要修复生产 loader，安全实现应当先只读解析并校验受系统管理的 AuthRoot CTL，再逐个应用 CTL 授权、时间窗口、禁用/Disallowed、现有 restricted DER、signature-hash deny 和严格 CRL 规则；不能打开 logical `Root` 作为替代。即使该步骤完成，也必须重新取得足够的离线 CRL/cache，才能跨过本实验暴露的 `UnknownRevocationStatus`。

当前本机的 CRL 材料确实不足以支持这两条公共链：默认 fresh snapshot 只有 `crls=1`；只读 `certutil -store -v CA` 看到的唯一 CRL issuer 是旧的 `OU=VeriSign Commercial Software Publishers CA, O=VeriSign, Inc., L=Internet`，`ThisUpdate=2001/3/24 8:00`、`NextUpdate=2004/1/8 7:59`，SHA-256 `35904127BF39E9DE2977DEB4C102AAB200C547FE5AD994EC7BD89EB20B435861`。它不对应 Cloudflare 的 SSL.com intermediate，也不对应 Google 的 `WE2` intermediate；AuthRoot 只读 variant 因而在补上 anchor 后稳定得到 `UnknownRevocationStatus`。这说明当前失败是本机离线信任材料和生产 loader 覆盖范围的缺口，不能靠 VM 才能定位，也不能用在线 CRL 获取或放宽 revocation 伪装成通过。

## 默认 native snapshot 证据

独立 fresh WMI 运行的只读诊断：`target/doh-native-snapshot-fresh.log`。

```text
readonly native snapshot: roots=10 intermediates=15 crls=1 deny_sha1=0 deny_sha256=0 deny_signature_hash=108 restricted=28
readonly native verifier: accepted
test trust::tests::readonly_native_snapshot_diagnostic ... ok
```

这证明默认物理 store/cache 可以构建 verifier，并没有因为空 CRL 或未支持的 cached signature-hash CTL 在 snapshot 阶段直接失败。它不证明当前 snapshot 覆盖公共服务链的 CRL，也不证明 Windows logical/Enterprise/Cryptnet trust 行为被完整复现。公共 TLS 查询当前直接因为缺少物理 ROOT trust anchor 报 `UnknownIssuer`，保持 No-Go。

## 门禁判断

已经验证：

- x64 与 x86/WOW64 的真实 product staticlib → MSVC DLL → C ABI 调用链；
- URL identity 与 literal IP 是分开传入的，探针没有使用 hostname resolver；
- fresh WMI 控制进程未加载 Aura Runtime；
- 默认 native trust snapshot 可加载并建立 verifier；
- 公共 IPv4 端点没有被错误地当成成功，失败原因保留为 typed certificate error；
- IPv6 端点单独执行并保留 network failure，没有修改宿主 IPv6 配置。

尚未验证或仍阻塞：

- 默认 native ROOT/CRL 下公共 DoH 的成功 TLS/HTTP/2/DNS response；当前 Cloudflare/Google IPv4 均为 `UnknownIssuer`；
- AuthRoot 物理 store 的安全接入：必须验证 AuthRoot CTL usage/policy、root program constraints、NotBefore/disable 与现有 Disallowed/signature-hash 规则，再决定是否纳入 snapshot；
- IPv6 public bootstrap；本机无 IPv6 route，另外的 IPv6 控制证据必须独立记录；
- 证书辅助流量、Host DNS/PAC/Cryptnet/AFD 的全机观测；本 probe 不是 ETW/WFP/抓包工具；
- 其他 Windows 版本、补丁和冷信任材料矩阵；
- 产品 Query Engine/Runtime router 的正式 DoH 接线、strict fallback、打包和生命周期验收。

本次结果不支持关闭证书校验、不支持在线下载 CRL 作为隐式 fallback，也不支持把 `error=6` 重新分类成成功。票 18 仍保持 No-Go，票 19 仍不可启用。
