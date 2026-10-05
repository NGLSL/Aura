# Container 实施进度与证据入口

Date: 2026-10-04
Status: independent-slices-implemented-and-reviewed
Review baseline: `8a971ab9b768412f0e8bbddbfea65c2b1966c0fb`
Branch: `dev`

用户授权按 implement Skill 推进全部 P0–P8，并在最后统一 code-review。49 张票的总目标保持不变；本文件只记录进度，不将计划或原型当作完整交付。

## 开始前已有工作

工作区已有 DNS 全 QTYPE 修复、Probe/CLI DNS 测试、Rust 1.99 toolchain、三份 CI workflows 与 README 修改；容器规格、研究及 DNS transports 规格也已存在。实施不得回退它们。Review 区分此前 DNS 工作与本轮 Container 新实现，不把原有改动当作未知作者的废弃文件。

## 本轮增量：混合架构恢复与 DoH 底座

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

当前缺 WDK/隔离 VM 的事实属于运行门槛；继续推进独立工作，不伪造驱动运行/正式签名证据。未提供的正式证书和产品签名资格仍归 43/44 验收，不提前阻断具有测试资格的原型。

用户已明确回复“尚无测试虚拟机，先完成可独立实现的部分”。因此不在宿主执行驱动加载、Verifier 或内核故障恢复；这些验收保持待完成，完整 P0–P8 总目标未达成。

统一 review 的 Standards / Spec 两轴都必须包含未完成要求，不能因为阶段代码测试通过就宣称 49 或完整 Container 已完成。实现后按 Skill 在当前分支提交经过 review 的具体变更；不隐含 push、release、宿主驱动安装或外部提交。
