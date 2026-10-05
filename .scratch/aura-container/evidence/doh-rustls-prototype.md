# DoH Rustls / Hyper 原型运行证据

Date: 2026-10-04
Related: [18 选型](../issues/18-ticket.md) · [19 正式接线](../issues/19-ticket.md)
Design: [本地信任与生命周期决策](doh-rustls-design.md)
Decision: **票 18 整体仍 No-Go；可交付本机双架构候选底座，不启用产品 DoH。**

## 已实现与实际范围

新 `crates/envbox-dns-doh` 使用 literal SocketAddr → Tokio TCP → Rustls → Hyper 低层 HTTP/1.1 或 HTTP/2。连接地址与 URL 的 TLS 身份、SNI、HTTP authority 分开；没有名称 resolver、系统代理/PAC、凭据和 redirect 路径。Rust staticlib 提供同步 C ABI，与既有 C++ packet transport 的绝对 deadline/cancel 契约对齐，尚未接入 Runtime router。

HTTPS POST 设置 Content-Type/Accept application/dns-message。HTTP 状态、媒体类型、Content-Encoding、header 与 body 有明确上限；最大响应 65535 bytes。首版连接池容量 0，每查询独占 runtime/连接；取消或失败后丢弃 request 并 abort/await 全部 owned Hyper task。最大任务数 64。接口返回错误类别，正式接线的审计映射尚未实施。DNS Question/ID/QR 等报文语义仍由正式接线时的现有 Query Engine 检查，HTTP 原型不替代它。

取消回调的 Rust 接口在 review 中修正：Budget 字段与内部方法 crate-private，安全 until 不携带回调，from_callback 是明确 unsafe 的有效期/no-unwind 构造契约。C ABI 保持 caller-owned buffer 与指针有效期契约；Rust panic 被隔离，OOM abort 不宣称属于可捕获 unwind。

TCP/TLS/HTTP await 成功后、send_request 前、每个 body frame 与最终返回前核对同一预算；同步 TLS verifier 返回后也先检查再发送 HTTP。最后增加 exchange 入口检查，避免 runtime 初始化耗过 deadline 后仍先连接。真实 loopback listener 回归先证明旧代码虽返回 Deadline 却建立了连接，修复后返回 Deadline 且 listener 无连接；`target/doh-exchange-entry-budget-{red,green}.log` 保留 RED→GREEN。

## 本机双架构 HTTP/TLS 夹具

`target/doh-prototype-fresh-green.log`：**56/56 通过**，x64 与 i686/WOW64 各 28 个场景，从 fresh WMI 控制进程运行。内存 test CA/CRL 仅由显式 fixture-trust feature 提供，不安装宿主证书，不通过产品 ENV 开启。

| 组 | 实际观察 |
| --- | --- |
| h2 / h1 | 同一受控 TLS 服务端记录 remote 127.0.0.1、SNI fixture.test、ALPN 与 authority fixture.test:实际端口；TLS 1.3 成功，另强制 TLS 1.2 的 h2 成功 |
| 资源 | 同一进程连续 16 次成功查询，x64 handles 94→94、x86 112→112；查询完成后服务端看到连接关闭 |
| TLS 拒绝 | 不可信、过期、错名、被吊销 EE/CA、显式禁止 EE/CA、无/错误 issuer/过期 CRL、非 serverAuth ROOT 均失败；证书负向没有 HTTP DNS request |
| HTTP 拒绝 | redirect、非 2xx、错误 media/encoding、声明长度超限与无 Content-Length 的 streamed 65536 bytes 明确失败 |
| 报文长度 | 65535-byte body 两架构成功；超过上限失败 |
| deadline / cancel | TLS 握手与 body 读取阶段分别覆盖总 deadline 与取消；Budget 接口修正后另行双架构定向复验，`target/doh-final-budget.log` exit 0 |

阶段检查修正后另有 fresh WMI 双架构 **10/10** 定向通过（每架构 h2 positive、握手/读取 deadline、握手/读取 cancel），`target/doh-final-stage-budget.log` exit 0。最终入口检查由上述真实 listener 回归及最终 workspace 验证；前序 56 个场景的产物没有被冒充为最后 source hash。

首次 x64 h2 HTTP15 的失败保留于 `doh-prototype-first64.log`：sender 过早释放；修复为持有 sender 到 body 完成，并正确等待 client cleanup。`second64` 的 25 场景通过只是早期直接执行，最终结果以上述 fresh WMI 的 56 个场景为准。旧 50 场景日志保留为 first-green，不冒充最终 56 项。

## 进程内网络观测

`tools/envbox-dns-doh-fixture/trap` 是单独测试 DLL。两架构原生 tripwire 实际证明它拦截了 DNS/getaddr、WinHTTP/PAC、CryptoAPI chain 等接口；唯一允许的 socket endpoint 是显式 literal TCP bootstrap。覆盖 connect、WSAConnect、WSAIoctl 返回的 ConnectEx 指针，并拒绝其他 endpoint、UDP 和未支持的 extension 路径。

56 个实际 backend 场景的 18 个禁止 API 计数均 0，socket denied/UDP 均 0；证书中 AIA/CRL 辅助地址的 canary listener 计数全部 0。正向服务端同时观测 remote IP、SNI、ALPN、authority、wire request，因此不是仅检查配置或 ALPN offer。

这些计数限定安装 trap 后的 client process 和已覆盖 Windows API；直接 AFD/NT、安装前缓存指针、其他进程和服务未覆盖。非提权 DNS ETW 启动被 0x80070005 拒绝，未安装驱动、启用事件频道或修改系统网络设置。因此 **不写成 WFP/ETW 全机抓包或所有 Windows 辅助流量为零**。预检与 tripwire 见 `target/doh-observation-preflight.json`、`doh-api-trap-check-results.json`。

## 本地原生信任与 C++ FFI

信任 loader 使用只读 physical registry ROOT/CA/Disallowed 与本地 CRL，以及本地 cached CTL；没有 SSL_CERT_FILE/DIR 覆盖、chain/URL retrieval。严格离线 revocation：空 CRL 拒绝，全非 root 链 Unknown Deny，过期 CRL 拒绝。支持 native signature-hash CTL，未知算法和策略继续拒绝；不宣称完整 Windows logical store/CTL policy 等价。

Fresh WMI Host 25796、Runtime modules=0：两架构各 5 个单位测试与 native snapshot diagnostic 通过。实际 ROOT 10、CA 15、CRL 1、signature-deny 108、restricted 28，标准 verifier 构建成功。首轮 Error11 原因是未支持缓存 signature-hash OID，修正后已成功读取；不能把它写成缺 CRL，也不能从存在一份 CRL 推断公共目标链已完整覆盖。

`tools/envbox-dns-doh-ffi-fixture` 另外把 **默认 feature 关闭的产品 staticlib** 链接进 MSVC /MD DLL，再由原生 EXE LoadLibrary/cdecl 调用。最终入口预算检查修复后的源码证据 `target/doh-native-ffi-final-entry-proof/` 包含源文件 hash、真实两架构 lib/DLL hash、native-static-libs 与 PE 日志；构建前后 `transport.rs` SHA256 均为 `CB09831DA7B664ECFD78F89496A87858BA53D1C1802A1C320C4747CDC149DB47`。fresh Host 21704 Runtime=0，native x64 PID 25524/x86 PID 4524 均 Runtime=0、exit 0。参数错误 1、取消 -1/错误 2且 callback 1 次、过期 deadline 3均符合 ABI；本地信任读取后，在各自独占绑定的非监听 loopback 端口（5202/5206）返回 Network 4。此前冻结源码证据目录保留原样，最终产物以 final-entry-proof 为准。

这证明真实混合语言链接、加载、调用约定和负向分类，不是原生系统信任的 TLS 正向。产品 staticlib 符号扫描没有 query_fixture/Snapshot::fixture；fixture bin required-feature，测试与产品使用不同 target 目录，cargo build --workspace 默认不会启测试信任。

## 门禁与剩余事项

此候选已证明当前本机 x64/WOW64 下成熟 HTTP/TLS 功能及所列观测路径；没有证明其他声明支持的 Windows 版本、完整系统流量，也没有原生本机 ROOT/CRL 的公共 DoH 正向。仅内存 CA/CRL 成功不能替代这些门槛。

因此不改 Runtime capability 的 dns_doh=0，不解除 Core 启动前 DoH 拒绝，不把票 19 写成完成。正式接线仍需通过 18 后完成 ordered bootstrap/fallback、Query Engine 校验、实际 Profile 注入、staging/installer 与生命周期验收。旧 WinHTTP No-Go 保留为不同候选证据，不能从本 Rust 原型推断它已合格。
