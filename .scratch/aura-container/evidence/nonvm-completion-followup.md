# 无 VM 剩余实现与验收跟进

Date: 2026-10-06
Baseline: `ca50ed3`
Branch: `dev`
Scope: 用户要求继续完成剩余实现；IPv6 按明确排期延期，不作为本轮门槛。

## DoH 原生门禁自动化

上一轮独立执行的 `config-policy-probe` 已进入 native `run.ps1` 的自动执行路径。fresh WMI worker 为双架构运行实际生产 decoder 的七项回归，检查进程 Runtime=0、全部断言与退出码；记录 path/SHA/exit/raw log，`config_policy_pass` 与 IPv4/TLS policy/process API 分项一起作为 wrapper 必须通过的门槛。

IPv6 默认不运行；结果写 `ipv6_deferred=true` 和四项明确延期记录。`-IncludeIPv6` 仅用于后续单独诊断。当前范围 `wire_positive` 不再受延期项影响；独立全局流量 observation gate 保留原边界，不冒充 WFP 保证。

完整默认 build→wrapper→fresh WMI run 实际通过：worker PID 20484 / Runtime 0，RunId `58f11a0d2c434b5cabc83ff5dcaff0b8`，completed=true、worker exit 0，native_ipv4/policy_negative/process_api/config_policy/wire_positive 全 true。x64/x86 config probe 各七项通过。

证据 `target/doh-native-config-automatic-wrapper.log`、`target/doh-native-acceptance-results-58f11a0d2c434b5cabc83ff5dcaff0b8.json`、对应 `target/doh-native-config-policy-58f11a0d2c434b5cabc83ff5dcaff0b8-{64,32}.log`。每个 run 保留唯一结果；stable latest 只是便于查看的副本。

## 本轮实现与验收队列

- 20 号统一 DNS：双架构四种 transport、多 QTYPE/解析 API、严格失败/顺序/取消/deadline，使用隔离配置和冻结 DLL；公共服务与受控 fixture 分别记录，公网超时不得隐藏。
- 12/13 号恢复：refresh 失败及时持久化 TrackingLost，保留原因/旧 sealed identity，不放宽未知 conhost 成员，不以路径授权或误杀。
- 05 号受控子进程：尚未验证的 WithToken child 明确拒绝，Hook 缺失拒绝 controlled init；Host/Compatibility 原有行为单独对照。
- 22/23 号后端准备：纯 C wire/identity/Host-Deny 核心、WDM adapter 及双架构 host harness。真实可信服务、WFP 接线和隔离加载资格仍缺，见 [控制面决定](kernel-policy-preparation.md)。

## DNS 统一矩阵与资源

新增 `tools/envbox-dns-unified-fixture/`，实际 CLI/Profile/Launcher/Runtime 运行。首次四传输矩阵 464 项中 460 通过、4 个公共 DoT deadline；受控 UDP/TCP 220/220、策略场景 24/24、公共 DoH 110/110。原 exit 1 保留于 `target/dns-unified-9c181ee113034ea79b9481aec5f77a85/result.json`（fresh WMI 13252 / Runtime 0）。补上 per-attempt frozen actor hash、实际 Runtime loaded、新审计 provenance 后，四个失败场景独立复验 4/4 通过，`target/dns-unified-repeat-9e9347aad6af4c9380fd6ea13e4025e9/result.json`（WMI 7036）。故意 expected hash 错误的负向在执行 query 前拒绝，结果 rows 0，`target/dns-unified-repeat-b9a4d371a30c4a0f8799a98f64f621c4/result.json`（WMI 19568）。

最终 latched pair 资源矩阵：10/10 资源稳定、8/8 实际父子 Profile/Instance/模块 hash 一致、72/72 async cancel 单次 callback 1223；704 次普通 TXT 中 703 成功，x86 DoT 一次 1460/deadline，完整 run exit 1 保留。证据 `target/dns-unified-resources-dba8d499f6954c5a88171f246159b507/result.json`（WMI 4072 / Runtime 0）。全部阶段 Host fallback audit 0，仅为被观测 API 路径，不是独立全局抓包。未无限重跑或放宽公共服务 deadline。GUI 退出后验证、更多 resolver 入口和生产长跑仍未闭合，20 号票不关闭。

## TrackingLost、停止失败与 bundle lease

refresh 的身份/成员/Job/lease 失败即时原子持久化 TrackingLost 与原因，保留最后 sealed identities。Stop 的实际 Job termination 失败保持 Lost；普通 refresh 不再覆盖 Running，失败请求重放返回 Partial，明确新 Stop 成功后才恢复停止流程。启动 cleanup 的 journal 写失败在返回错误与内存原因中明确报告，旧 sealed journal 不损坏。

bundle recovery 复用最终稳定 file lease 单次读取并遵守四秒共享 cooperative deadline；拒绝网络/device/relative/reparse 路径，实际 retained handle identity 重验，不新增路径授权。同步 OS 调用无法硬中断，返回后再检查期限，不能宣称硬实时四秒上限。真实 active named Job、query-only handle AccessDenied、锁 journal、junction 与 lease 检查均有定向证据。最终 Supervisor lib 13 passed/0 failed/0 ignored，fresh WMI 29168 / Runtime 0，`target/tracking-stop-cleanup-reviewed-{result.json,tests.log,tests.err.log}`。OS reboot 与完整 conhost 矩阵仍未验。

## WithToken 受控边界

新增 CreateProcessWithTokenW required Hook；controlled 下调用前明确返回 ERROR_NOT_SUPPORTED 50，Host/Compatibility 保持原 API。受控必需 process Hook 从 3 升至 4，IPC identity validation 同步要求 4。controlled 状态初始化一次锁存，清除/修改环境变量不能降级，Compatibility 也不能后续升级。

实际双架构 mutation RED 6 项失败（旧 pair）、GREEN 6/6，通过 fresh WMI 24320，`target/with-token-latch-green-result.json`；最终基本 Host/Compatibility/Controlled 6/6，WMI 4540，`target/with-token-latched-basic-boundary-result.json`。所有 controller Runtime 0。这是 unsupported boundary 的实际拒绝证明，不是 WithToken 子进程正向支持。Probe cleanup 检查实际 Terminate/Wait 错误，不把未知存活子进程报告为已清理。

最终 Runtime pair `target/with-token-latched-runtime/`：

```text
x64 74DF21C7867A95B7DBF8EE6D625614BC00CCA1147B6A43504A0447EDB3F1CC20
x86 064490B6A831B9F2F3C24D0B052C0674D6DA96713896A13418A1A72739E34C22
```

## 最终集成验证

Rust 1.99.0 `cargo build --locked --workspace` 和 `cargo test --locked --workspace --no-fail-fast` 均 exit 0，427 passed / 0 failed / 34 ignored；fresh WMI 29096 / Runtime 0，使用上述最终 latched pair。日志 `target/workspace-container-final-{build.log,test.log,result.json}`。前一次 process Hook contract 未同步导致 DLL init 0xC0000142 的失败记录 `workspace-nonvm-closure-*` 保留，修复为四项后取得成功，不拼接旧日志。

全套后 Standards 提出原路径→canonicalize TOCTOU，修复为先原路径 stable handle、实际 canonical file ID 比对，最后仅原 handle 一次 hash。受控 seam 实际相同字节不同文件 ID 的替代目标被拒绝，原文件在解析时已禁止写删；没有声称真实并发竞赛。最后源码 SHA `06D5503F7E99C83E5B1C99D232DEDEF6896E32275657F81C7555C96632614511`，fresh WMI 6740 / Runtime 0：Supervisor lib 14/0，混合架构恢复 4/0（x64→x86 与 x86→x64，各 root live/exit），两者 exit 0。原始记录 `target/recovery-container-final-result.json`、`recovery-container-final-unit.log`、`mixed-container-final.log`、`mixed-container-final-host.log`。这次定向覆盖修后代码，不将先前全套结果称为最终新源码全套。

修改 Rust 文件 rustfmt、四个 PowerShell AST、三个 Python AST 和 git diff 检查通过；双架构 Runtime/Probe/native harness 构建各项结果独立归档。统一审查见 [review](nonvm-completion-review.md)，Standards 六项、Spec 三项、WDM 文档一项均修复后独立复核；完整规格的剩余资格不能因 review 无 actionable 而算完成。

## 后端与交付边界

纯 C 核心 x64/x86 每架构 157 assertions 通过，实际 C++ 链接检查通过；WDM adapter 的 x64 SYS 编译链接通过。未加载驱动、未配置测试信任或服务、未改宿主网络；Native clone/PSS 覆盖仍为正式加载/Container 资格门槛。真实 WFP、可信控制服务、文件/Registry 后端、安装升级和 VM/Verifier 未完成。以上源码和验证不等于完整 P0–P8 交付，IPv6 不计当前阻塞。
