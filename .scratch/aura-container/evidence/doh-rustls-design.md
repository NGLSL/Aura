# DoH 显式连接与离线 TLS 候选

Date: 2026-10-04
Related: [18 选型门禁](../issues/18-ticket.md) · [19 正式 transport](../issues/19-ticket.md)
Baseline: `9919ad1200e2406f1059e5a577a4a965cdd57ab7`

## 决策与当前资格

WinHTTP 原型保留为 [No-Go 的既有证据](doh-bootstrap.md)。本轮验证另一个成熟库候选：Rust staticlib 内使用 Tokio literal `SocketAddr`、tokio-rustls 与 Hyper 低层 `client::conn`。没有 resolver、代理/PAC、重定向或通用 HTTP 客户端。HTTP/2 由成熟库处理。是否正式接入以原型结果为准；研究本身不能解除票 18 门禁。

选择依据是可明确控制 I/O 的边界：Tokio 的 `SocketAddr` 转换不执行名称解析；Rustls 不自行执行网络 I/O；Hyper 接受已经建立的 TLS stream。URL 主机名保留为证书身份、适用时的 SNI 和 HTTP authority，连接地址单独传入。[Tokio 地址转换源码](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.53.2/tokio/src/net/addr.rs)、[Rustls I/O 契约](https://docs.rs/rustls/0.23.43/rustls/index.html)、[Hyper HTTP/2 Connection](https://docs.rs/hyper/latest/hyper/client/conn/http2/struct.Connection.html)。这些源码契约支持原型设计，不能代替本机运行和辅助请求观测。

## 本地信任材料

不用 `rustls-native-certs` 的公用加载入口：它受 `SSL_CERT_FILE/SSL_CERT_DIR` 覆盖；Windows loader 也有属性读取 `unwrap()`。专用 loader 只读固定本地 store，所有读取失败返回 typed error，不调用 CryptoAPI chain、SSL chain policy、URL retrieval 或网络获取。[公用加载入口](https://raw.githubusercontent.com/rustls/rustls-native-certs/main/src/lib.rs)、[Windows loader](https://raw.githubusercontent.com/rustls/rustls-native-certs/v/0.8.4/src/windows.rs)。

`READONLY` 不会把 logical system store 收窄到本地 registry；logical collection 可包含额外注册 physical provider。候选使用 `CERT_STORE_PROV_SYSTEM_REGISTRY_W`，明确 CurrentUser/LocalMachine 的 ROOT、CA、Disallowed。此选择缩窄原生 logical store 的覆盖，不宣称完全复现 Windows Enterprise/SmartCard/CTL 链策略。[CertOpenStore](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certopenstore)、[System Store Locations](https://learn.microsoft.com/en-us/windows/win32/seccrypto/system-store-locations)。

- ROOT 才能作为 trust anchor；CA 只提供中间证书候选，不能把中间证书提升为独立信任锚。
- ROOT 转换前检查有效期和 Windows 有效 EKU。`CertGetEnhancedKeyUsage(flags=0)` 的零 OID 必须区分 `CRYPT_E_NOT_FOUND`（全部用途）与成功错误码 0（没有用途）；非空列表必须允许 serverAuth，读取失败不能当作不限用途。[EKU 契约](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certgetenhancedkeyusage)。
- WebPKI anchor 不保留 Windows 属性 EKU/CTL，所以筛选必须在转换前完成。[WebPKI trust anchor 源码](https://raw.githubusercontent.com/rustls/webpki/v/0.103.8/src/trust_anchor.rs)。
- Disallowed 同时约束 ROOT、叶证书和全部中间证书候选；wrapper 只能增加拒绝条件，必须完整委托标准链、名称和 TLS handshake signature 校验。
- 仅枚举 Disallowed 完整证书不能覆盖 hash-only CTL。须明确支持的 CTL 标识算法、正确解析并测试；未知形式不能静默忽略，也不能声称完整原生 distrust 等价。[Windows chain 与 CTL](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certgetcertificatechain)、[CTL 枚举](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certenumctlsinstore)。
- Disallowed store 的 CTL 枚举还不能覆盖 AuthRoot 自动更新已落地的缓存。候选额外只读 `HKLM\SOFTWARE\Microsoft\SystemCertificates\AuthRoot\AutoUpdate\DisallowedCertEncodedCtl` 的有界 REG_BINARY，使用 CertCreateCTLContext 解码并共享 SHA-1/SHA-256 subject 标识检查；不存在与存在但损坏/不可读必须分开处理。本机研究实际看到该缓存，不能仅靠 store 枚举推断完整。此读取不更新缓存、不触发在线获取。[Windows certificate trust 缓存说明](https://learn.microsoft.com/en-us/windows-server/identity/ad-cs/certificate-trust)。
- 枚举先复制 DER，后续枚举会释放 previous context；提前返回须释放当前 context。仅正常结束错误可当作完成；每轮检查取消/deadline，并限制总数量和字节。[枚举释放契约](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certenumcrlsinstore)。

## 吊销策略

保持现有 DoT 的严格离线策略，不为 DoH 偷换为“缺材料时跳过”：先拒绝空 CRL，标准 verifier `with_crls` 加 `enforce_revocation_expiration`，保持全非根链深度和 Unknown Deny。不启用只查叶证书或 unknown allow。无匹配 issuer、过期、不支持格式和 revoked 均失败，不补做 URL 获取。[Rustls verifier 源码](https://docs.rs/rustls/0.23.43/src/rustls/webpki/server_verifier.rs.html)。

Rustls 空 CRL 会跳过吊销，默认也允许过期 CRL；其标准 verifier 不验证 stapled OCSP。store 内 CRL 不等于完整 Cryptnet/OCSP 缓存。因此本机材料不足时公共 DoH 可以失败，不能把内存测试 CA/CRL 的成功写成产品公共服务正向。此限制必须在交付中明示。

## FFI、HTTP 与资源

同步 C ABI 借用输入到返回为止，输出写入固定容量 buffer；任何 panic 都不能越过 C++ 边界。URL 与 literal IP 使用各自有界字段。请求使用 HTTPS POST 与 `application/dns-message`，不跟随 redirect。响应检查状态、媒体类型、header/body 上限；DNS 的 ID、QR、opcode、Question 等继续由已有 Query Engine 检查。[RFC 8484](https://www.rfc-editor.org/rfc/rfc8484.html)。

每次查询拥有 current-thread runtime 与连接，首版池容量为 0；所有 bootstrap、信任读取、connect、TLS、HTTP header/body 共用 C++ 绝对 `GetTickCount64` deadline。取消/超时后停止尝试上游，abort 并等待连接和内部任务销毁，不能只丢弃 JoinHandle。同步枚举/证书验证不宣称任意指令可抢占。[Tokio JoinHandle 契约](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.53.2/tokio/src/runtime/task/join.rs)。

静态库分别构建 x64/i686 MSVC，关闭 Rustls 默认 features，显式 ring/std/tls12；与 C++ 对齐 CRT，按真实 `native-static-libs` 输出链接。只导出必要 C 接口，x86 使用 cdecl。两架构 Rust 编译不能代替 C++ DLL 链接运行。[Rust linkage](https://doc.rust-lang.org/reference/linkage.html)、[Rustls provider](https://docs.rs/rustls/0.23.43/rustls/index.html)。

## 必要实测与证明边界

测试信任入口只存在于 standalone fixture feature，不写宿主 trust，不通过产品 ENV 开启。受控同一连接记录 remote IP、SNI、ALPN=h2、HTTP authority/path 和 DNS wire。负向包含不可信、错名、过期、吊销、无 CRL/错误 issuer/过期 CRL、Disallowed、错误状态/媒体、redirect、超大/中断/慢速 body，以及各阶段 deadline/cancel 和重复查询资源回落。

fresh WMI Host 须确认 Runtime modules=0。进程内 tripwire 观测并阻断 DNS/getaddr、WinHTTP/PAC、CryptoAPI chain/URL API 和非配置 socket endpoint；受控 AIA/CRL/OCSP/redirect canary 记录实际访问。每项观测限定到覆盖范围，API trap 不能冒充系统抓包，内存 CA 正向不能冒充 production loader 正向。冷加载、两架构和目标 OS 单独归档；无法观察或关联的部分保持 Unproven/No-Go，不自动启用产品能力。

本文件记录研究与实现约束。实际日志、锁定依赖版本和最终 Go/No-Go 由后续原型证据补充，不能从本文推断票 18/19 已完成。

## Windows 原生禁止记录的签名 hash

实际本机缓存的 CTL 并非普通 DER SHA-1/SHA-256：SubjectUsage 为 `1.3.6.1.4.1.311.10.3.30`，SubjectAlgorithm 为 `1.3.6.1.4.1.311.10.11.15`（szOID_DISALLOWED_HASH），20 个条目的 SubjectIdentifier 均为 16 bytes。仅支持前两种算法时，快照真实返回 TrustSnapshot 错误 11；这不是缺 CRL，原失败必须保留。

此 OID 的精确契约对应 Windows `CERT_SIGNATURE_HASH_PROP_ID`（15）。候选对每个 ROOT/CA/peer DER 创建临时 certificate context，并读取该本地属性；不把条目猜成 DER MD5，不调用链构建或在线获取。按 CTL SubjectIdentifier 的原始 byte array 比较，不颠倒字节。官方 header 定义与属性文档为依据：[Microsoft wincrypt.h](https://raw.githubusercontent.com/microsoft/win32metadata/main/generation/WinSDK/RecompiledIdlHeaders/um/wincrypt.h)、[CertGetCertificateContextProperty](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certgetcertificatecontextproperty)、[CertFindSubjectInCTL](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-certfindsubjectinctl)。

研究的本机只读校验对 75 个 LocalMachine ROOT DER 比较 property 15 与直接 CryptHashToBeSigned，75 个全部相等；返回长度含 16/20/32/48/64 bytes。因此不硬编码为 16 或 20 bytes，使用有界可变长度标识，未知算法和读取失败仍 fail closed。这是本地算法契约验证，不是 TLS 正向或全机网络观测。TLS 签名和链校验继续由标准 WebPKI 完整执行；旧 hash 仅用于附加拒绝，不允许弱 TLS 签名。

## 锁定源码核对

当前 Cargo.lock 锁定 rustls 0.23.45、rustls-webpki 0.103.15、Tokio 1.53.1、Hyper 1.11.1、ring 0.17.14。本地 registry 源码复核确认：空 CRL 跳过、默认过期允许、全链 Unknown Deny、OCSP 未验证、anchor 属性丢失和 SocketAddr 无解析等关键行为与上述研究版本一致。

Hyper 1.11.1 允许丢弃 send_request future 来取消 h2 stream，发送 RST_STREAM(CANCEL)，但共享连接仍可继续使用。因此当前每查询独占连接的 FFI 必须另外结束 connection driver 并释放 socket；stream 取消不能当作整个连接已关闭。
