# DoH 标准 TLS 与 Runtime 接入修复

Date: 2026-10-06
Baseline: `a0dbac904865769915a16b501babe66d7f6e7362`
Branch: `dev`
Scope: 修复公共 DoH 的默认信任阻塞，并接入 Profile / Runtime 的实际 DNS 数据面。

## 授权与策略变更

用户在确认根因后明确要求“那就修复吧”。此前把 DNS strict 与“完整且已知的离线吊销状态”绑定为 DoH 默认使用门槛；本次显式拆分，不把该策略变更伪装成旧严格规格的通过。

每个 DoH upstream 增加 `tls_revocation = "standard" | "strict_offline"`。Standard 缺省，始终验证可信链、身份、用途、有效期、证书签名和 TLS 握手签名；现有 CRL 的 known revoked 仍拒绝，不要求完整 CRL 覆盖/新鲜度，不进入 CDP cache collector。StrictOffline 保留全非根链 known status / fresh CRL / cache-only retry，缺材料仍失败。DNS strict 独立禁止 Host fallback；运行时仍不支持 `strict=false`。DoT 既有吊销策略未改。

固定版本 `webpki-root-certs = 1.0.9` 提供 Mozilla 公共 DER 根，与只读本机物理 ROOT/CA/Disallowed 共同构建应用信任。完整 DER 通过已有证书 hash / signature hash / restriction 过滤，不提升 CA 中间证书、不全量导入 AuthRoot、不使用在线 provider。公共根更新随依赖和应用发布，不在查询时下载。数据许可证随 `THIRD_PARTY_NOTICES.txt` 交付。[Rustls 官方根证书说明](https://github.com/rustls/webpki-roots)、[固定版本 API](https://docs.rs/webpki-root-certs/1.0.9/webpki_root_certs/)。该策略不宣称复现全部 Windows Enterprise / GroupPolicy / CTL trust semantics。

TOML 缺省兼容为 Standard；typed Runtime DTO 必須显式 `dns_upstream_N_tls_revocation=0|1`；immutable v2 DoH snapshot 也必须显式绑定字段和 digest，旧缺字段 DoH snapshot / 未知值拒绝恢复。旧 DoH 当时不可运行；v1 和非 DoH v2 结构不变。GUI selector / 编辑 / 排序 / 保存和 CLI `--tls-revocation` 均保留显式 StrictOffline。C ABI 原入口缺省 Standard，新增 `envbox_doh_query_with_policy` 接受 0/1，其他值在 trust/network 之前返回 Argument。

## 实际数据面

C++ Runtime 将 DoH endpoint 交给 Rust staticlib，返回 DNS packet 后复用已有 QNAME/QTYPE/ID/Question 校验和 Windows record 转换。只尝试配置的 literal bootstrap；URL 为 literal IP 且无 bootstrap 时直接用该 IP。bootstrap/upstream 重试使用同一绝对 deadline；取消停止重试。无 Host DNS、PAC、继承代理、重定向或隐藏明文上游。

Runtime CMake 必须构建并链接 default-feature-off / locked Rust staticlib，每个架构/CMake tree/config 使用独立产物；能力报告 `dns_doh=1`。Win32 MSBuild 的架构环境仅在 Cargo 子进程清理；C++ Debug CRT 与 Rust /MD 对齐。x64/x86 Release、x64 Debug 和增量 CMake 构建均成功，未运行安装器或修改宿主配置。

## Google 的第二处根因：HTTP/2 Host

标准信任修正后 Cloudflare 首先成功，Google 则稳定返回 HTTP 15。独立最小产品调用取得 `hyper::Error(Http2, Reset(StreamId(1), PROTOCOL_ERROR, Remote))`。相同 Google TLS/HTTP2 测试中，`:path=/dns-query` + 无普通 Host 成功，有 Host 则 reset；Cloudflare 两种都接受。产品绝对 URI 本身正确，Hyper 会生成 `:scheme/:authority/:path`。因此仅 HTTP/1.1 保留普通 Host，HTTP/2 由绝对 URI 生成 `:authority`，没有强制退为 HTTP/1.1。

原始证据 `target/doh-google-diagnostic/{direct.err,direct.out,compare-http-host-header.log,compare-http.py}`。新增 loopback H2 assertion 的 RED 使用修复前真实 fixture binary：fresh WMI PID 10400 / Runtime 0，observed Host `fixture.test:12106`、主线程 assertion、exit 1；不是后台异常被忽略。GREEN fresh WMI PID 30292 / Runtime 0，双架构 **114 场景通过**（112 IPv4 TLS + 2 invalid CLI），40 个 H2 请求场景共 70 请求全部无普通 Host，`:authority` 精确匹配 URL，H1 两场景仍通过。原 StrictOffline 与新 Standard 正负集完整保留。IPv6 10 项 preflight 10013 未执行。

GREEN 日志 `target/doh-http2-authority-reviewed{.log,-host.json,-result.json}`、`-build64.log`、`-build32.log`；RED 为 `target/doh-http2-authority-red-confirmed{.log,-host.json,-result.json}`。最后 fixture SHA-256 x64 `2F9A3745557B33027DA0B823EE337A4A8807014AD8FA9F841E7A35CF4D81D42C`，x86 `635D04FFBBB732985A992EB577B9881483D94EAFEB0B3C34AE0A682973483650`。

## 默认 native C ABI 与进程 API 观测

最终 RunId `85c40f0502dc42fba1ce52216c888850`，fresh WMI PID 25356 / Runtime 0；两个架构各 7 个独立 Host，共 14 进程，全部 Runtime 0、exit 0。wrapper / worker exit 0；`native_ipv4_pass`、`policy_negative_pass`、`process_api_pass` 均 true。

| 场景 | x64 / x86 实际结果 |
| --- | --- |
| Cloudflare / Google Standard IPv4 | error 0，61-byte DNS，ID a042 / QR 1 / RCODE 0 / 完整 question match |
| Cloudflare / Google StrictOffline IPv4 | error 7，响应 0，缺吊销材料按策略拒绝 |
| 未知 policy 2 | error 1，所有 API/socket 0 |
| 两服务 IPv6 | 各架构 executed 0，共 4 项未执行 |

每个 Host 在加载产品 DLL 和 trust snapshot 前安装 process-local trap。Standard/Strict IPv4 只允许 1 次配置 literal TCP connect，其余 API 0，denied endpoint/extension 0；未知策略及未执行 IPv6 计数全部 0。覆盖 18 项 Host DNS/PAC/Windows chain API、UDP 与连接扩展，共 24 API，不覆盖 Cryptnet URL API、直接 AFD/系统调用、其他进程或全局流量。

来源 `target/doh-native-acceptance-results-85c40f0502dc42fba1ce52216c888850.json` 及相同 RunId 64/32、逐 case 日志。宽泛的 `acceptance_pass/gate` 仍 false 表示完整全局/OS 验收未证明，不能把 native IPv4 分项成功翻译成完整 Container 保证。

| 最终原生产品产物 | SHA-256 |
| --- | --- |
| x64 probe MSVC DLL | `0AAFFCD942A80237FF84F905693365BB377EFF58633A8C462399026A0A4E6F6F` |
| x86 probe MSVC DLL | `2B9E4D7E86DA2C209E5E506283F31B13949A5BDCECC828CFC29BE634AD0BB636` |
| x64 staticlib | `72165EFBDFD66CB1BB34C572FE2E391ED793F0F8635E0C9E1A2AA1CC26FD11A0` |
| x86 staticlib | `A28CD4A4725CBA27D83C42F7667AA764332BDB2E1718C2CCCC27E78AFA358E2D` |

## Profile → 注入 → Runtime Hook 实测

初版冻结 pair 的双架构 24 项检查通过，涵盖 A/AAAA/HTTPS65/TXT/MX/PTR、SVCB64 权威 NODATA、getaddrinfo、严格离线拒绝、bootstrap retry/all-dead 和受控 TLS stall 取消。证据 `target/doh-integrated-behavior-assertions.json` / `-behavior-result.json` / `-cancel-result.json`；旧 hash 32DC6C… / D1DC65… 只属于修复 H2 Host 前的来源，不当作最终 Google 通过。

最终 reviewed pair 双架构 **16 项通过 / 0 失败**：Cloudflare/Google A/HTTPS65、StrictOffline、受控 bootstrap retry、all-dead、TLS stall async cancellation。成功查询返回原生 record 并正常释放；SVCB 无记录不是 transport 失败。取消 pending 9506 → cancel call 0 → completion 1223，审计 `doh-cancelled`。严格材料不足/all-dead 返回 1460。所有项 Runtime loaded，逐实例没有 `dns-host`/fallback 审计；该结论为 Runtime 审计，不冒充独立抓包。

fresh WMI 控制器 PID 11812 / 5296，入口 Runtime 0；受控监听器均核对自己 PID/命令行并结束。来源 `target/doh-integrated-reviewed-assertions.json` / `-behavior-result.json` / `-cancel-result.json`，配置/审计目录 `target/doh-reviewed-profile-d4a643b0-447c-46cd-8d4a-11543fb74141` 与 `target/doh-reviewed-cancel-profile-2f4d9ac1-8dcc-4ad5-82ce-1254cfafc6ae`，均为隔离测试配置，没有修改用户 Profile。

HTTP/2 修复阶段的 Runtime pair `target/doh-integrated-runtime-reviewed/`（后续 strict guard 最终版本见下文）：

- x64 `05712B125A462F753E33F4374DFF84319E7CB01852A1EC044341554DB48910ED`
- x86 `08215208A7BB8291E9EF12721F064012064F9E6B36CC8461C0E901C0B6D7BCE0`

构建日志 `target/runtime-doh-integrated{64,32}-authority-reviewed-build.log`。最终 Profile 测试明确按目标架构设置匹配的 DLL；显式指定 x64 ENVBOX_RUNTIME_DLL 不被误认为自动选择 x86 的证据。

## 剩余范围

当前 Windows 上标准 TLS 的公共 IPv4 DoH、正常失败策略及实际注入已经分项证明。IPv6 保持用户关闭设置，其他 OS build、完整全局流量观测、安装/升级/发布未验证。StrictOffline 缺材料仍失败；真实 AuthRoot policy/provenance/rotation 的研究未被当作默认使用前提，也未被声称已完成。驱动/Verifier/WFP/overlay/内核恢复仍需隔离 VM，完整 P0–P8 未完成。

本切片 review 见 [双轴审查](doh-standard-tls-runtime-review.md)。

## 最终审查修复：原生 strict 启动门控

Spec review 发现 Rust launcher 虽拒绝 VirtualView `strict=false`，Runtime 共享配置 decoder 却会接受原生 ENV/IPC 的非 strict 配置。直接编译生产 decoder 的新 `config-policy-probe` 取得 RED：fresh WMI PID 2412 / Runtime 0，x64 probe PID 25364 / Runtime 0，七项中两项失败，exit 1。两项都属于 VirtualView strict=0/false 被接受。

在共享 decoder 的 strict 解析后、读取上游前加入 `dns_mode == 1 && dns_strict != 1` 拒绝条件。IPC/ENV 均使用此 decoder；Host strict=0 仍允许，legacy 默认 strict=1。GREEN fresh WMI PID 22732 / Runtime 0，x64 probe PID 10248 / x86 PID 17820，均 Runtime 0；双架构各七项通过，exit 0，含 Standard/StrictOffline 完整 wire 往返、缺失/未知 policy 拒绝、VirtualView 两种非 strict 拒绝和 Host 兼容。

证据 `target/doh-config-policy-red64{.log,-result.json}`、`target/doh-config-policy-green{64,32}.log` 和 `target/doh-config-policy-green-result.json`。测试链接实际生产 decoder，不重写一份等价实现。GREEN EXE SHA-256 x64 `C9B08BDC1A25C4EE235D3CE0114B14667241987909C1E7E2B699F5DA781BA500`，x86 `03E3C3B7A0ACBDA841B34C0545A61A99ECAAE3F1659BF3187B07AF06DB97B647`。

最终 strict guard Runtime pair `target/doh-integrated-runtime-final-guard/`：

- x64 `0B9BDB5A3925001CC5F7755D1D44FD4C2226A5136C5D5752071E60F23D8E25D1`
- x86 `401A24D7536D5E5AC490462E8F6678664A46CC229BC9B328502166BE2ECCE8E7`

两次 Release 重建 exit 0，日志 `target/runtime-doh-integrated{64,32}-final-guard-build.log` 确认重编译 decoder。旧冻结 pair 和日志保留。

最终 guard pair 再次完成真实 Profile 注入 **16 passed / 0 failed**，双架构 Cloudflare/Google A/HTTPS65、strict、retry、all-dead、受控 TLS 停滞取消均符合前述结果，Runtime loaded / fallback audit 0。fresh WMI 控制器 PID 27188 / 26172、入口 Runtime 0；重复使用已构建隔离客户端。证据 `target/doh-integrated-final-guard-{assertions,behavior-result,cancel-result}.json`；配置/审计目录 `target/doh-final-guard-profile-a132c2ff-6c60-4a01-a24a-b7c46b3b1f2f` 和 `target/doh-final-guard-cancel-profile-e24587c3-dbf9-4afd-a64a-86f35426d292`。该复验绑定上述最终 SHA，未覆盖旧证据或修改用户配置。

首次全量工作区 build exit 0 / test exit 101，仅 `runtime_pipe_auth::versioned_dns_wire_preserves_order_and_rejects_partial_snapshots` 使用缺少新 policy 的旧测试 DTO 导致失败。已同步显式字段，并补充缺失/未知 policy 的拒绝断言；该测试单独通过。

最终完整回归重新绑定 guard pair：fresh WMI PID 22656 / Runtime 0，Rust 1.99.0 `cargo build --locked --workspace` / `cargo test --locked --workspace --no-fail-fast` 均 exit 0，46 组结果合计 **418 passed / 0 failed / 34 ignored**。来源 `target/workspace-doh-standard-reviewed-{result.json,build.log,test.log,controller.log}`，起止 UTC 10:07:09.9829468Z–10:09:12.9489887Z，结果 JSON 含最终双架构 DLL 路径与完整 SHA。

DoH crate x64/i686 的实际单独测试各 **26 passed / 0 failed / 1 ignored**。诊断工具的独立 locked cargo check exit 0，保留原有 29 项 dead-code warnings。已修改 Rust 文件以 `rustfmt +1.99.0 --check --edition 2021 --config skip_children=true` 通过；避免递归触碰基线未格式化的其他模块。两个 PowerShell 脚本和 Python fixture AST 检查通过。native config probe 是独立执行的回归，不冒充 run.ps1 自动 public acceptance 门禁。
