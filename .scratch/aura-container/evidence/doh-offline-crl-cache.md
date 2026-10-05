# DoH 原生 CRL 缓存只读切片

Date: 2026-10-06
Baseline: `af1f6f681cd523c79709c69677870db6334865d8`
Branch: `dev`
Status: implemented / partial-acceptance / ticket-18-No-Go
Related: [18](../issues/18-ticket.md) · [研究设计](doh-rustls-design.md) · [AuthRoot 研究](authroot-offline-research.md)

## 结果与范围

新增 `crates/envbox-dns-doh/src/offline_crl.rs`，补充 CA physical store 之外的 Windows 用户 Cryptnet CRL cache。只读 `certutil -urlcache CRL` 实际列出约 80 个条目，包含 Google 相关路径；CA store 的旧 CRL 观察不能代表全部本地缓存。列表使用无 URL、无 `-f`、无 `delete` 的显示操作，没有下载或写入材料。[certutil URLcache 契约](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/certutil)、[缓存与 store 区别](https://learn.microsoft.com/en-us/windows/win32/seccrypto/certificate-revocation-list-semantics)。

没有全量导入 AuthRoot、没有新增系统 chain/代理/Host DNS fallback，也没有启用正式 DoH transport。产品仍使用原有固定 physical ROOT/CA/Disallowed；AuthRoot 变体只属于独立研究诊断。票 18 No-Go、19 阻塞及 P4–P8 VM 验收保持。

## 实现契约

- 从 detached certificate extension 提取 CDP，固定 `CRYPT_GET_URL_FROM_EXTENSION`，不枚举未记录的 cache 文件目录。
- 仅接受 HTTP(S) cache key；拒绝 file/LDAP/FTP、userinfo、fragment、控制字符和反斜杠。限制证书 64 份、CDP 32 个（包括重复输入）、URL 4096 UTF-16 units、URL buffer 512 KiB、输入证书与输出 CRL DER 合计 8 MiB。所有超限/畸形/非正常读取错误失败关闭。
- `CryptRetrieveObjectByUrlW` 的 object 为单个 `CONTEXT_OID_CRL`，flags 固定 `CRYPT_CACHE_ONLY_RETRIEVAL | CRYPT_DONT_CACHE_RESULT`；不加入允许 wire retrieval 或 cache 写入的分支。正常 cache miss 返回无候选，其余错误拒绝。[官方 flags 和对象所有权](https://learn.microsoft.com/en-us/windows/win32/api/wincrypt/nf-wincrypt-cryptretrieveobjectbyurla)。
- context 由 RAII 释放；二次 URL buffer size 与 embedded pointer/string 均检查范围；DER 复制后才释放 context。
- cache DER **未认证**。native verifier 仅在 unknown/expired revocation 时读取候选，随后使用相同 ROOT 和标准 verifier 再验证 issuer、签名、freshness、全非根链 revocation、名称和用途。已吊销、未知 issuer、错名、签名或用途失败不重试。
- Rustls 空 CRL 可关闭 revocation，因此空列表绝不能产生接受结果。fixture 构建时拒绝；native 等收到 CDP 后找缓存，仍无材料则拒绝。fixture 信任不访问宿主 cache。
- 读取前后检查绝对 deadline，CAPI 使用有限 timeout；同步 CAPI 无法任意指令抢占。verifier 仅保存 deadline，transport 在 TLS 完成后、HTTP 前及任务销毁后检查 caller cancellation，不把 caller callback/context 延长为 verifier 的生命周期。

**仍未支持的启用门禁：native cache 阶段无法在各 CAPI 步骤间及时响应 caller cancellation。** deadline-only verifier 不读取该 callback；取消后仍可能继续本地 cache 工作直到返回或 deadline。transport 随后检查取消并禁止 HTTP/后续上游，但这不能等同于 cache 阶段完整取消支持。需用生命周期可证明的取消状态补齐，再解除正式 transport 门禁；本轮不增加 unsafe Send/Sync 或把借用 callback 延长到 Arc verifier 生命周期。

每次缓存读取和原 snapshot 各自有独立 8 MiB DER 上限，组合仍有界。缓存当前包含 OCSP 或 delta/partitioned CRL 并不表示 Rustls 已支持这些材料；不能把它们当作普通完整 CRL 跳过检查。大量 unrelated CA/CDP 超限也会保守失败，未证明跨 Windows store 规模的兼容性。

## 实际运行

原生诊断直接复用生产 Rust source module，并通过 fresh WMI 创建 Runtime 未注入的 Host。observer 对 presented-chain CDP 额外做一次只读采样，只打印候选数量、bytes/hash，不把它们作为已认证材料或改变 verifier。

| 运行 | Host PID / Runtime modules | x64 与 x86 的 IPv4 结果 | 身份 |
| --- | --- | --- | --- |
| 默认信任，RunId `1f90ceaebf154742b98fdaa0b26cc118` | 30468 / 0 | Cloudflare `UnknownIssuer`；Google `UnknownIssuer` | product source 默认 loader；非正式 C ABI 验收 |
| AuthRoot 研究变体，RunId `b38522b922b7448680d1c3eb99723271` | 28996 / 0 | Cloudflare cache hit 0；Google cache hit 2；两者最终 `UnknownRevocationStatus` | 添加 AuthRoot 候选的研究 binary，不能升级生产信任 |

尺寸修正与 fixture 候选分支增加后的最终诊断，默认 RunId `56317ca35d87400fb0c9fa62a984ce1f` / Host PID 1988，研究 RunId `2c77fed9738843b6b07871d36cab412d` / Host PID 20044；两者 Runtime modules 0、worker exit 0，双架构结果与上表一致。旧 unique RunId 结果均保留。

Google 两份未认证候选（两个架构一致）：

```text
529 bytes   SHA256 39C727D70B8BDF5FC324233D0823555EB66CE846119609A22D1CEBEDBC3CF80C
1871 bytes  SHA256 6F7BFA4789BD506CCF81640FEEC036BAC7C0354084CA472F55A79CE53B9C3ADE
```

日志位于 `target/doh-native-diagnostic{,-authroot}-results-<RunId>.json` 及同 RunId 的 `-64.log`/`-32.log`。唯一 RunId 原始结果保留；stable aliases 只指向最近一次。较早无 observer 的本轮默认/研究 RunId `e0b4f74ca5ea476ebe28194d4e919926` / `b7103a54f6614519a87a5b4d59f3a719` 也保留。

以上只证明 cache 读取和严格失败；未定位精确缺失的链节点/CRL 分片，不声称公共 TLS 正向或全系统无辅助流量。IPv6 TCP 为 OS 10051，用户已禁用 IPv6，不计入 TLS 证据，未改宿主。

## 验证

- `cargo +1.99.0 check -p envbox-dns-doh --locked` 通过。
- i686 实际执行 crate tests：14 passed / 0 failed / 1 ignored。包括 6 项 cache reader 测试、native empty-CRL/cache-miss 必须拒绝、retry allowlist；生成测试证书只解析 context，不安装、不作为真实 TLS 正向证据。
- cache miss 使用实际 CAPI，并以 owned loopback listener 检查零连接。拒绝非 HTTP key、pointer 范围/溢出、坏 DER、输入上限，以及 adapter 收到已到期/已取消的 Budget 时的入口拒绝有负向覆盖；后者不证明 native verifier 的 deadline-only CAPI 阶段能及时响应 caller cancellation。
- 审查修正前的历史 fixture 结果：两架构重新构建；fresh WMI Host PID 2632 / Runtime modules 0；`target/doh-offline-crl-matrix.log` **58 个已执行 IPv4 场景全部通过**，exit 0。错误身份、过期/吊销/未知/错误 issuer CRL、全链 revocation、Disallowed、HTTP/媒体/redirect/body、deadline/cancel 等保持严格验证。fixture trap、canary 均按原矩阵断言。最终结果为下一项的 84 场景。
- 审查后新增 **13 个缓存重试场景/架构**，总计 **84 个 IPv4 场景通过**；最终 fresh WMI Host PID 30664 / Runtime modules 0，`target/doh-offline-crl-reviewed-matrix.log`、host/result JSON，exit 0。包括有效候选与过期材料刷新正向、miss、错误 issuer、过期/篡改签名、revoked EE/CA、完整链正向/缺 CA 撤销、不能覆盖已知 revoked、错名和不可信链。`with_fixture_cached_crls` 仅编译到 test/fixture-trust；显式提供生成的 DER 候选，进入同一个重试/标准验证分支，**不读写真实 URL cache**。这些正反向证明重试分支不会忽略校验，不能冒充原生 cache 公共信任正向。
- IPv6 预检失败，10 个 IPv6 场景未执行；不计入通过。
- 审查修正前的历史 workspace 结果：fresh WMI Host PID 16056 / Runtime modules 0，Rust 1.99 build/test exit 0，**403 passed / 0 failed / 34 ignored**。日志 `target/workspace-offline-crl-final-{build,test}.log`、result JSON；沿用未修改的 `target/nonvm-final-runtime-v2` pair，SHA256 与上一切片一致。本次新增逻辑在 DoH Rust crate；完整 Runtime 的正式 DoH 能力仍未开启。
- 审查修正前的历史默认 product staticlib → MSVC x64/x86 DLL → C ABI 验收：RunId `bf65409ffcc34e9c9d024c23c77d17a2`，fresh WMI Host PID 26960 / Runtime modules 0，双架构构建/链接/host exit 0。Cloudflare/Google IPv4 均 error 6 certificate、无响应；顶层脚本 exit 1 为预期 No-Go，`gate=false`，不误记为构建失败或公共正向。原始 unique JSON/log 为 `target/doh-native-acceptance-results-<RunId>.json` 和同 RunId 两架构日志。
- 尺寸修正后的最终 whole-suite：Host PID 17372 / Runtime modules 0，Rust 1.99 build/test exit 0，仍 **403 passed / 0 failed / 34 ignored**；`target/workspace-offline-crl-reviewed-{build,test}.log` 与 result JSON。最终 i686 crate tests 14/0/1，`target/doh-offline-crl-tests32-reviewed.log`。默认 C ABI 最终 RunId `61b07c27a4a84e339f2bba5935e62201` / Host PID 9548 / Runtime modules 0，双架构构建/链接/调用完成，公共 certificate failure 与 No-Go 保持。此前日志与结果未覆盖。
- 最终双轴 review 在 [本轮审查](doh-offline-crl-review.md) 单独记录。

## 剩余工作及下一门槛

1. 依据 AuthRoot 研究实现 detached signed CTL 的明确 signer-store 验签、freshness/sequence/未知属性负向 fixture。production 接入必须同时具备 signer 来源/轮换与 root policy 的可解释证据，不复用只解码 CTL 的 deny parser 来授权信任。
2. 对所需公共链明确缺失的 CRL/CDP，区分普通 CRL、delta/分片与 OCSP；先验证现有只读来源，再规划独立维护阶段的受控材料输入。Runtime 不在线补材料。
3. 在受控信任正向通过后补 HTTP/DNS/C ABI 正向、完整系统网络观测和目标 OS 矩阵，才评估解除 18/19。
4. 驱动加载/Verifier/故障恢复仍需隔离 VM。它不阻止前述用户态研究，但完整轻量 Container 尚未实现/验收完成。

本轮没有安装/加载驱动，没有改网络、IPv6、证书仓库、系统 trust 或注册表配置，没有 push/release。
