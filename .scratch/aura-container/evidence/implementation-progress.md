# Container 实施进度与证据入口

Date: 2026-10-06
Status: independent-slices-implemented-and-reviewed
Review baseline: `a5baed8`
Branch: `dev`

范围修订（2026-10-06）：用户明确容器是 Profile 环境信息视图，旧 P0–P8 存储/权限隔离总目标已被[当前规格](../spec.md)取代。下方阶段、缺口和“完整容器”措辞保留为历史执行记录，不再作为新目标的完成门槛；已有 DNS、快照、监管和恢复成果继续复用。最新服务/WFP源码与测试证据见[增量记录](service-wfp-followup.md)，未部署实验代码保持未启用。本次修订不将旧49票标完成，不清除既有失败证据。

## 当前状态摘要（2026-10-06，优先于下方历史记录）

- 已实现用户态工作区/不可变快照、Supervisor/Job 监管、受控启动与子进程传播、双架构及混合架构恢复；真实应用/特殊启动入口和故障恢复矩阵仍有未验项。
- DNS 已接入 UDP/TCP/DoT/DoH、任意 QTYPE 与 strict 无 Host fallback；DoH 公共 IPv4 和最终 Runtime 双架构真实 Profile 验证通过。此前“18 No-Go / 19 off / dns_doh=0”是历史状态，已由提交 `5e8f929` 的修复取代。
- 本轮 Rust 1.99 工作区验证：427 passed / 0 failed / 34 ignored，最终 latched 双架构 Runtime，fresh WMI 29096 / Runtime 0。随后审查发现 bundle 原路径与 canonical path 身份的竞态，补充修复及定向恢复验证单独记录；不把前一全套日志冒充修后全套。见 [本轮跟进](nonvm-completion-followup.md)，忽略项不算通过。
- 2026-10-06 用户明确决定 **IPv6 延期，当前不实施，不作为当前阶段完成、IPv4 DNS 交付或后续独立工作的阻塞项**。既有未执行记录保留，只表示 IPv6 未验证，不表示 IPv4 实现失败。AAAA QTYPE 支持与 IPv6 上游连接是不同维度，不因延期删除 AAAA 解析。
- 本轮新增四传输统一 fixture、实际父子与资源检查、TrackingLost/Stop 失败持久化、bundle lease 约束及受控 WithToken 明确拒绝。受控 UDP/TCP 和公共 DoH 通过；公共 DoT 的限时失败原始记录保留。GUI 退出、独立全局 capture、真实应用/管理/恢复缺口和生产长跑仍未闭，详见本轮证据。打包/安装/发布单独记录。
- 完整轻量容器仍缺：有效 WFP 网络策略接线、真实可信控制服务、文件和 Registry 隔离后端、基于私有状态的数据管理、驱动签名/加载/Verifier/故障恢复及最终安装兼容性资格。本轮已新增 WDM 控制设备/实际进程对象归属 adapter 与策略核心，x64 SYS 编译链接通过；尚未加载、不提供实际过滤或完整 Container。Native clone/PSS 覆盖是正式加载资格门槛；存储规则预检不是强制隔离。见 [内核准备证据](kernel-policy-preparation.md)。

以下各段按历史来源保留；当前 DoH 和 IPv6 排期以本摘要及最新证据为准，不从历史未通过状态重新阻塞已经交付的能力。

## 开始前已有工作

工作区已有 DNS 全 QTYPE 修复、Probe/CLI DNS 测试、Rust 1.99 toolchain、三份 CI workflows 与 README 修改；容器规格、研究及 DNS transports 规格也已存在。实施不得回退它们。Review 区分此前 DNS 工作与本轮 Container 新实现，不把原有改动当作未知作者的废弃文件。

## 本轮增量：无需 VM 的独立实现与验收

用户要求先完成不依赖虚拟机的部分，随后确认宿主禁用 IPv6。本轮保持 IPv6 关闭；IPv6 场景的未执行不计入通过，且不作为其他 IPv4 失败的解释。完整 P0–P8 和 49 票总目标仍未完成。

| 范围 | 本轮实际结果 | 剩余门槛 |
| --- | --- | --- |
| 05 真实入口矩阵 | 旧 V3 pair 的 CreateProcess/AsUser、CMD、PowerShell 输出 Runtime/Profile 且实际 exit 0；稳定 process handle generation 核对/停止控制样本通过，临时根清理成功 | Host Chrome 与 Aura Chrome 同参数都提前 exit 13，浏览器 Unverified/renderer NotObserved；WithToken、Native、Packaged 等仍未验 |
| 09 管理通道负向 | same-SID low IL 真实访问 medium endpoint 被 OS 以 error 5 拒绝，正常 manager 同 generation Ping 成功；不存在 endpoint 为 error 2/helper 29，不能误算通过 | 其他 SID、remote、高完整性矩阵与服务端收到请求后的 low IL 拒绝仍未验 |
| 12–13 恢复/Runtime 保留 | 恢复失败写回 TrackingLost journal；NoJob 即使最后 sealed generations 均不存在也不冒称完整树 Exited；稳定 read-only/read-share file handle 验证 file identity 和有界 SHA，实际写入/删除/释放后篡改拒绝 | conhost、OS reboot、Supervisor 崩溃后的外部 cleanup、installer upgrade 等仍未闭 |
| 16 异步 Resolver | ExW 的 event/callback/cancel、提前关闭原 event、callback 释放 caller storage、80 次无需 helper 的 event 退休通过；DnsQueryEx callback 内取消旧 token 返回 87，同 storage 重入后旧 generation 不能取消新请求；不支持的 provider/namespace/flags 明确拒绝 | fixture 证明 Profile 路由与生命周期；全机 Host DNS 零流量及应用自带 DNS 绕过未证明 |
| 17 公共 DoT | 冻结 v2 Runtime 的 x64/x86 strict Profile IPv4 单次运行，A/W/UTF8/Ex/async 五入口均成功；fresh Host 无 Runtime，live module path/hash 与指定 DLL 相符，原始输出及唯一 JSON 保留 | 旧 V3 的 x86 async 1460 与历史重试保留；本轮单次 smoke 不证明系统辅助流量和跨 OS/缓存矩阵 |
| 18 原生 DoH | 双架构本地 IPv4 58 个场景通过；IPv6 10 个场景未执行。product-default native FFI 公共 IPv4 为 UnknownIssuer；独立 AuthRoot 研究变体暴露下一层 UnknownRevocationStatus | AuthRoot CTL 严格安全接入、离线吊销材料、完整系统观测及其他 OS 未证明；整体 No-Go，19 未启用 |
| 21 驱动构建 | 固定 hash WDK/SDK NuGet 输入、真实 x64 空功能 SYS、INF/CAT/PE 结构检查通过；明确 unsigned | 没有安装/加载；测试信任、VM/隔离环境、Verifier、恢复与正式签名仍缺；22 加载门禁不变 |

分项证据见 [真实入口](real-app-independent-final.md)、[恢复与保留](recovery-independent-final.md)、[管理负向](management-independent-final.md)、[异步 Resolver](resolver-async-final.md)、[DoT](dot-independent-final.md)、[DoH IPv4/IPv6](doh-ipv6-independent.md)、[DoH 原生信任](doh-native-acceptance.md)、[空驱动构建](driver-build-independent.md)。下面前次结果保留其原始来源，不混作本轮验证数。

本轮最终检查采用 fresh WMI Host PID **19324**，`Runtime modules=0`，清理继承的 `ENVBOX_*` 后使用冻结 v2 pair：

- Rust **1.99.0** `cargo build --locked --workspace`：exit 0。
- `cargo test --locked --workspace --no-fail-fast`：**395 passed / 0 failed / 34 ignored**，exit 0；ignored 不计入通过。包含最新 32 项 DNS CLI 测试、callback 内旧 token 取消/re-entry 断言、恢复与 bundle lease 测试。
- 原始日志 `target/workspace-nonvm-final-build.log`、`target/workspace-nonvm-final-test.log`；身份/退出码/时间/双架构 hash 见 `target/workspace-nonvm-final-result.json`。
- MSVC/CMake x64/x86 Release Runtime 构建通过；本轮 8 个修改的 Rust 文件直接 `rustfmt --check` 通过；PowerShell 8 个脚本和 Python 2 个脚本解析通过，`git diff --check` 通过。未扩大整理既有全仓格式差异。
- 最终 Spec review 修复 callback-free Probe 自身的释放后读取竞态；即时响应 fixture 及重建 x64/i686 Probe 后，最终 suite 再次通过。fresh WMI PID **11012**、Runtime modules=0 的 x86 mixed 注入定向测试 **6 passed / 0 failed**；新 Probe/hash/原始日志见 Resolver 证据。
- 首轮通过结果（Host 13116）保留于 `target/before-probe-review-workspace-nonvm-final-*`。其后将 x86 运行与构建并行造成 `envbox.exe` 被占用、构建 error 5，失败日志和 result 保留为 `workspace-nonvm-final-concurrent-build-failure*`；已在 x86 测试结束后顺序构建/运行取得上述最终结果，未把旧 test log 与失败 build result 拼成通过。
- [本轮统一 review](nonvm-unified-review.md)：Standards、Spec 分别 **0 个未处理 actionable finding**。运行验收与剩余门禁独立记录，review 通过不表示全部 49 票完成。

本轮最终 Runtime pair 为 `target/nonvm-final-runtime-v2/`：

```text
x64 CA8283ADAE000DBEAAE65902A10F2E0E05B94C38C14F7B3652EDD3066C43D965
x86 5D2FD1038748F0D579F5B2EB59EBAEECDFF6C7846D12FD93876875696B6CEBE7
```

该 pair 用于最终 workspace/DNS 与公共 DoT 证据；真实入口、驱动、原生 DoH 和早期 recovery 证据各自注明实际产物，不将旧产物的行为结论移植到新 DLL。全套检查不包含安装器实际安装/升级、正式发布、驱动加载或 VM 验收。

## 前次增量：混合架构恢复与 DoH 底座

上次交付为 `9919ad1`；用户明确优先补混合架构恢复，再推进 DoH。完整 P0–P8 仍未完成，本轮新增结果如下。

| 范围 | 已实现及实际证据 | 剩余门槛 |
| --- | --- | --- |
| 12 mixed recovery | schema 3 逐成员 generation/实际 Runtime path+SHA/configSHA/version；成员增长认证后原子记录；两个架构方向 × root live/exit 四例各两次真实 crash/restart通过；旧 schema 2 两例恢复、旧 mixed 拒绝接管，共 7 native 实例通过 | OS console companion 未有 Runtime identity，实际 conhost 场景仍 Lost；NoJob/未知完整树、OS reboot 未闭 |
| 18 DoH 候选 | Tokio literal SocketAddr + Rustls + Hyper h1/h2；本地 physical trust/CRL/Disallowed cached signature-hash；x64/WOW64 56 个 HTTP/TLS 场景通过，修后 10 个 deadline/cancel/positive场景通过；两架构 product-feature-off C++ DLL 真正链接和调用 | 整体 No-Go：原生 ROOT/CRL 公共目标正向、IPv6、其他 OS 与完整系统观测未证；19 未接线，产品 dns_doh=0/Core 拒绝仍保留 |
| Broker lifecycle | 全套发现 stop nudge 早于下一 pipe 创建的 lost-wakeup，程序已退出但 CLI join 永久等；确定性 RED 0.26 秒，发布后重验 stop 修复；Broker 5/5 与真实 auth 7/7 GREEN | 首轮人为 once-nudge 才继续，明确为 assisted，不冒充无缺陷全绿 |

最终 **388 passed / 0 failed / 33 ignored**，未注入 WMI Host 10452，无额外唤醒，81.21 秒，`target/workspace-mixed-doh-head.log` 与 result JSON；最终 locked workspace build 通过。新增 ignored 包含显式 native fixtures，不计通过；所列 7 个恢复场景和原型证据另行真实运行。Standards/Spec 最后分别 0 个未处理 actionable finding。

详见 [恢复证据](mixed-recovery.md)、[DoH 原型](doh-rustls-prototype.md)、[研究设计](doh-rustls-design.md)、[本轮统一 review](mixed-doh-review.md)。下面保留上次交付的事实，不把历史 378 项结果当作本轮最终检查。

## 前次交付切片（9919ad1）

用户已明确没有测试 VM，先完成可独立实现的部分。以下是新增实现及实际证据；各票仍有未验证验收，不预先关闭，也不表示完整轻量容器已完成。

| 范围 | 已实现及验证 | 仍缺验收 |
| --- | --- | --- |
| 02–05 身份/门禁/传播 | 真 PID generation、进程 token、实际模块及 SHA、完整 immutable DTO、必要 Hook 集；五类 console/GUI entry x64/x86；24 组 A/W/AsUser × 父子架构 × 挂起；CMD 四组合；TLS 入口明确拒绝 | 真实 Chromium、WMI/Native/服务代理/WithToken；跨用户/IL/remote；Packaged 实机及并发激活所有权 |
| 06–08 工作区与快照 | 持久 UUID CRUD；严格带摘要快照与旧 schema 读取；GUI/CLI 只经 Supervisor Run/List/Stop；headless 管理状态测试 8/8，GUI build | 安装后的可见 GUI、完整 storage backend、Container/Strong 模式 |
| 09–11 Supervisor | 独立 same-SID/exact-IL 管理通道、可信 application/snapshot UUID；Run 幂等和原子 Job 创建；Stop/StopAll 作用域与原始目标集合重放；真实 A/B/Host 保留 | 另用户/IL/remote 负向、安装升级与长时间资源边界 |
| 12–13 恢复与能力 | named Job 的 Runtime 只读 query handle escrow；完整 nonce challenge 重连；实际 Supervisor kill/restart 的 A/B 和 root-exited-child-live 两例通过；损坏 A 为 Lost、B 仍可 Run/Stop；staged DLL DATA capability 在 Create 前校验；短命 identity 与退出缓存双架构通过 | 混合架构子树恢复、OS reboot/Job 不可重开且未知成员的终态 |
| 14–16 严格 DNS | typed ordered UDP/TCP/DoT/DoH DTO；任意 QTYPE；共享 deadline/cancel；required Hook 失败拒 init；Raw/未支持 async 同步拒绝；真实 UDP 精确 endpoint 网络策略 6 sends；最终 full suite DNS 28/28 | 完整零 Host 流量抓包、尚未实现地址异步输入 |
| 17 DoT | literal IP + TLS 1.2 + hostname/IP SAN；本地 chain/revocation 失败关闭；独立 fixture 32/32；实际注入 20/20 失败/显式 UDP fallback；16 次 handles 无增长 | 产品可信 DoT 正向、CryptoAPI 辅助流量抓包；无池、同步 chain 校验不可中途取消，整票未完成 |
| 18–19 DoH | 双架构 WinHTTP bootstrap prototype 实际 literal IP/URL authority/SNI/h2/TLS 错误验证；正式门禁 NoGo，配置可存但启动明确拒绝 | CryptoAPI 辅助解析隔离与 OS 矩阵未证明；19 正式数据面未实现 |
| 21/26 | driver 只读预检；存储四动作规则校验与词法预览，can_authorize=false | 无 WDK、测试 VM、正式签名材料；无 file/Registry overlay 或强制后端 |

安装器 staging、NSIS install/uninstall 及 artifact manifest 已包含 Supervisor；仅静态 manifest 检查，不表示安装器实际运行。统一 review 已修复旧入口身份确认、包装遗漏、Lost 隐藏、LastError 顺序、child cleanup 假 Exited、恢复无界文件读取及默认 DNS 明文域名审计。

## 最终检查与固定产物

- `cargo +1.99.0 test --locked --workspace --no-fail-fast`：**378 passed / 0 failed / 25 ignored**，exit 0，89.01 秒；fresh WMI Host `RuntimeModules=0`。日志 `target/workspace-independent-final-verified.log` 与 result JSON。ignored 不计通过；门禁/身份/恢复所需冻结 bundle 测试另有实际定向证据，浏览器/交互/特定环境的剩余项仍未验。
- `cargo build --locked --workspace` 通过；MSVC/CMake Runtime x64/x86 通过；changed Rust files `rustfmt --check`、`git diff --check` 通过。全仓 `cargo fmt --all --check` 仍报告未修改的 picker/window/browser-probe 等既有格式差异，未扩大改动整理。
- [统一 review](unified-review.md)：Standards / Spec 分别 **0 个未处理 actionable P1/P2**，含最后 legacy child 绑定修复的增量复核。不能从该结论推断未实现后端或验收已完成。
- 原 whole-suite Git 两项失败保留于 `workspace-independent-final.log`；V2 wrapper repeat 14/20 空 stdout，V3 相同 capture **32/32 GREEN**；native nongated same/mixed/suspended **6/6 GREEN**。根因修复为完整 IPC child 在首次 Resume 前 sealed REGISTER_CHILD，不改变 gate/caller suspension/授权。
- `workspace-independent-final-v3.log` 的 channel 并发测试失败保留。真实 reason 为两个测试占同 principal 端点却配置不同 artifact；只修改 fixture 端点互斥，内部四客户端仍并发；`--test-threads=2` 连续 5 轮 **2/2 GREEN**，最终 whole suite 通过，未放宽生产认证或 timeout。

最终冻结 pair：`target/container-independent-final-runtime-v3/`（本地忽略的 build artifact）：

```text
x64 215c42dc101265a45343c8f599acdd9de2aabe999578ecf60011f81c54146d36
x86 438d56e88e73eb0d2fca87400ec2f134c937041555c709611bdb2857be0f7e1a
```

前序 typed16 v1/v2、independent-final/V2 pair 及失败日志未覆盖。DoT 独立 fixture、原实际 TLS negative/fallback、DoH NoGo、Supervisor recovery、跨架构子进程各自有不同固定 pair/hash，不把先前 evidence 冒充最终 DLL 的全量重跑；最后 whole suite/legacy child 使用上述 V3。

交付为当前分支经过 review 的独立切片与持久规划。无 push/release/驱动安装；安装升级、可见 GUI、正式签名、OS reboot 和完整 P0–P8 不在本次已验证完成范围。

## 早期检查点（保留失败来源，以上最新切片优先）

| 票 | 状态 | 验证/阻塞 |
| --- | --- | --- |
| 01 | 产品修复及正常回归通过，故障验收部分阻塞 | Launcher 106 passed/3 ignored；fresh WMI host CLI Probe 23/23。初版四组 32/64 fixture 各 33 次句柄无增量，但旧代码也通过，非 RED；扩展故障 fixture 已编译，独立 Host CreateProcess error 5 导致未执行。后续 cleanup-green 日志已被失败重跑覆盖，不引用为原始绿色证据 |
| 06 | 配置 CRUD 已实现，定向检查通过 | Storage 5/5、实际 CLI 1/1、headless GUI state、Core/Storage/App 102 passed/5 ignored、GUI/CLI build；可见安装 GUI 未验证。旧 ANSI acceptance 在 fresh WMI Host RuntimeModules=0 下单项通过 |
| 21 | 只读报告与脚本完成；实验资格阻塞 | [预检证据](driver-preflight.md)：MSVC/SDK 存在；WDK、明确测试 VM 与测试信任材料未提供，空驱动编译及 22 实际加载未完成。正式资格独立阻塞 43/44 |
| 02 | 身份与授权切片通过，整票验收未完成 | 真 pipe RED→GREEN；6 passed/2 explicit fixtures ignored；实际 x64/x86 Runtime 独立模块路径、磁盘 SHA256、配置身份及必要 Hook 数确认。非 owner/remote、真实 PID 重用与全部 Hook 故障注入未证明 |
| 03 | Runtime 门控实际 32/64 通过，Launcher 接线验证中 | success/wrong bundle/disconnected/idle Host/TLS 拒绝均有实际入口 marker；初始挂起保持。显式 start_session_gated 仅收到身份及 client ACK 后发布 Running；当前 console 无 EXE TLS 范围，普通 CRT/GUI 待验证；导入 DLL 初始化不是入口门控覆盖范围 |
| 04 | 子进程受控登记实施中 | generation、真实父身份、架构匹配 immutable bundle 与绑定 ACK；不把 Job 成员自动当作已验证 Runtime |
| 07 | 不可变快照配置切片通过，真实启动接线中 | Core/Storage/App 111 passed/6 ignored；实际 CLI 2 passed/1 ignored。旧 Profile 修改/删除/损坏不影响已准备快照；摘要、schema、超限、身份、完整字段严格校验；Rust x86 DTO 执行未证明 |
| 09 | 独立管理通道通过，业务接线中 | fresh WMI Host RuntimeModules=0；公共 client 测试 2 passed，ignored helper 由四个独立管理进程实际执行。实际 token/image/hash、协议/generation、目标拒绝、并发单实例、deadline/cancel；另用户/IL/remote 实机负向未证明 |
| 10 | Supervisor 启动事务实施中 | 快照与 Application 由 Supervisor 可信 Storage 读取；显式 gated startup、Job/Session 持有、实际 generation 登记，不允许未实现 Container 模式 |
| 14–15 | typed 配置与 UDP/TCP 共用 transport 实施中 | 保持旧 IP 顺序迁移与完整快照 schema 校验；未实现 transport 在启动前拒绝，不能因字段丢失切 Host |
| 18 | DoH bootstrap 隔离原型中 | 先证明 literal IP + URL authority/SNI/证书身份 + 实际连接/协议；无隐式宿主 DNS 证据不足时保留正式实现门禁 |
| 26 | 配置/预览切片通过，强制 backend 未实现 | Core/Storage/App 106 passed/5 ignored；显式实际 C/D fixed NTFS 配置验证通过。只有 IsolatedWrite 单卷要求；词法预览 can_authorize=false；尚无文件/Registry Overlay |
| 其余票 | 尚未实施 | 按 map 的依赖推进，不预先关闭 |

## 验证与完成纪律

真实连接身份、Runtime 身份和 Hook 完成状态分开确认。旧 Runtime 的 READY 发生在 Hook 安装之前；新实现于 Hook transaction commit 后报告完整身份。Detours 初始挂起没有 Runtime ACK；入口门控使用真实标记及 x64/x86 证明，不在 loader lock 内等待放行。

WDK/SDK 构建输入已于 2026-10-06 在项目 `target` 内恢复并通过空驱动构建。当前仍缺明确的隔离 VM 与测试信任/恢复路径，不伪造驱动运行或正式签名证据。未提供的正式证书和产品签名资格仍归 43/44 验收，不提前阻断具有测试资格的原型。

用户已明确回复“尚无测试虚拟机，先完成可独立实现的部分”。因此不在宿主执行驱动加载、Verifier 或内核故障恢复；这些验收保持待完成，完整 P0–P8 总目标未达成。

## 2026-10-06 后续：DoH 只读 CRL cache 与 AuthRoot 研究

基线 `af1f6f6` 之后新增 Cryptnet CRL cache-only/no-write adapter 和 native 严格 revocation 重试；fixture 不读宿主缓存。修正早期材料结论的范围：CA store 只有旧 CRL 不代表用户 Cryptnet cache 没有材料，只读列表实际约 80 条。双架构 presented-chain 观察：Google cache hit 2，Cloudflare 0；研究 AuthRoot 变体仍 `UnknownRevocationStatus`，default 仍 `UnknownIssuer`。未知/过期、空列表和 cache miss 保持拒绝，未在线补取或放宽 trust。

审查修正前的历史结果：fresh WMI Host PID 16056 / Runtime modules 0，Rust 1.99 workspace build/test exit 0，**403 passed / 0 failed / 34 ignored**；i686 DoH crate 实际 **14 passed / 0 failed / 1 ignored**。两架构重新构建的 IPv4 fixture **58/58 passed**，Host PID 2632 / Runtime modules 0；IPv6 disabled，10 项未执行。default staticlib/MSVC DLL/C ABI 双架构构建和运行完成，但公共 IPv4 均 certificate failure，18 No-Go/19 off 保留。最终审查后结果见后段的 84 场景与 Host PID 17372 whole-suite。

AuthRoot 签名/显式 signer trust、Root Program 属性、freshness/rollback/未知 schema 以及受控离线材料输入的后续顺序已经保存，不能把全量 AuthRoot 提升为 root 来通过验收。证据和剩余实施门槛见 [缓存切片](doh-offline-crl-cache.md)、[AuthRoot 研究](authroot-offline-research.md) 与 [本轮 review](doh-offline-crl-review.md)。无 VM 的用户态研究可继续；驱动加载、Verifier、完整 WFP/overlay 和故障恢复仍保留原门禁。

本轮审查修正 CDP 二阶段实际长度小于预估值时的误拒，全部 pointer 检查改用实际返回范围；新增 26 个 fixture cache 候选重试正反向场景，最终双架构 IPv4 **84/84 passed**，WMI Host PID 30664 / Runtime modules 0。修正后完整 suite 再验 **403/0/34**，Host PID 17372 / Runtime modules 0，build/test exit 0；默认 C ABI 最终双架构仍 certificate failure/No-Go。native cache 各步骤及时取消尚未支持，已明确作为正式启用前门禁，不以 fixture 取消通过代替。

统一 review 的 Standards / Spec 两轴都必须包含未完成要求，不能因为阶段代码测试通过就宣称 49 或完整 Container 已完成。实现后按 Skill 在当前分支提交经过 review 的具体变更；不隐含 push、release、宿主驱动安装或外部提交。

## 2026-10-06 后续：native cache 取消与受控 signed CTL

基线 `b193fe9` 之后补齐上一切片保留的 native cache 步骤间取消门禁：caller-thread 非 Send RAII scope 保管完整 Budget，Send + Sync verifier 只保存不可复用的 thread/scope identity；foreign/retired scope 不调用 callback，外部 callback 前释放 registry borrow。首次取消锁存并在所有任务清理后优先返回 typed error，一次性 cancel pulse 不被 TLS General 错误吞掉。单个同步 CAPI 内部不可抢占。

双架构实际 native cache-only collector 的入口、CDP 返回后、retrieval 返回后取消均通过；查询后 callback 恢复零，HTTP/canary 仍为零。包含原回归的 **92 个 IPv4 场景通过**；fresh WMI PID 4932 / Runtime modules 0。IPv6 10 项仍因用户禁用而未执行。

新增 test/fixture-only signed CTL 受控子集：显式 DER signer pins 的 memory store，逐 signer index 真实验签，然后检查 usage/list identity/time/sequence/subject。未知属性严格拒绝；equal sequence 需要之前 encoded SHA-256；没有生产 AuthRoot 导入或持久化 rollback state。签名、篡改、多 signer、EKU、时间、算法、rollback 和 bounds 正反例双架构实际通过。Standards 审查发现的两阶段 CAPI actual-size/embedded pointer/OID 范围缺口已修正并加纯 helper 测试；Spec 没有新增 actionable finding。

默认 staticlib/MSVC DLL/C ABI fresh WMI 复测仍 certificate failure，默认诊断 `UnknownIssuer`、研究 AuthRoot 变体 `UnknownRevocationStatus`。Google cache 候选 2、Cloudflare 0，仍不能证明完整吊销。真实 AuthRoot policy/provenance/rotation、公共离线信任和完整观测门禁不因受控 fixture 通过而解除；18 No-Go/19 off。完整回归、来源/hash 与审查见 [本切片证据](doh-signed-ctl-cancellation.md) 和 [双轴 review](doh-signed-ctl-cancellation-review.md)。P4–P8 驱动/Verifier/内核恢复仍需隔离 VM。

最终修正后 fresh WMI PID 5584 / Runtime modules 0，Rust 1.99 workspace build/test exit 0，**414 passed / 0 failed / 34 ignored**；x64/i686 DoH crate各 **25 passed / 0 failed / 1 ignored**。Standards 原 P2 已经独立复审标记 resolved；Standards/Spec 当前新增未解决 actionable findings 均 0，完整规格仍 partial。

## 2026-10-06 用户授权修复：标准 TLS 与真实 DoH Runtime

本轮基线 `a0dbac9`。用户明确确认问题后要求修复：每个DoH上游的 `tls_revocation` 与 DNS strict 拆开，Standard缺省完整校验证书但不要求本机完整CRL，StrictOffline保留材料不足失败；固定MozillaDER根包提供明确应用信任，不全量AuthRoot提升。GUI/CLI/IPC/不可变快照均传递策略。Runtime必链接default-feature-off Rust staticlib，DoH接入既有任意QTYPE Query Engine并报告真实能力。

补充发现Google HTTP2会因额外普通Host头而RST_STREAM(PROTOCOL_ERROR)。保留absolute URI自动生成:authority，仅H1发送Host；产品最小错误和独立有/无Host对照定位，旧binary本地assert RED → 当前双架构114场景GREEN。最终默认native Cloudflare/Google IPv4都成功，strict与未知policy拒绝、process24 API分项通过；IPv6仍明确未执行。

最终真实Profile/Launcher/Runtime双架构16项关键验收通过，含两个公共服务A/HTTPS65、StrictOffline、受控bootstrap retry/all-dead与async cancel；先前24项广覆盖包括AAAA/TXT/MX/PTR/SVCB/getaddrinfo保留。此次独立且不需VM的DoH可用性已证明，安装升级、完整OS/globaltraffic/IPv6及内核运行保证仍未证明。证据、最终回归数字和审查见 [修复证据](doh-standard-tls-runtime.md)、[双轴review](doh-standard-tls-runtime-review.md)。未发布或修改宿主网络/trust。
