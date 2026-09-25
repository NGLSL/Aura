# Status: ready-for-agent

## Problem Statement

EnvBox / Aura 已能把一个进程树的 Locale / Region / Language / Timezone / DNS View / Environment 约束成一套 Environment View。但用户在真实浏览器场景里仍会看到：

1. **HTTP 出口与 WebRTC 出口不一致** — 浏览器走代理后 `claude.com` 出口是境外 IP，`RTCPeerConnection` ICE/STUN 仍可能打出宿主真实公网地址（例如广州住宅 IP）。这是网络路径泄漏，不是「读到的地区值不对」。
2. **DNS View 不等于网络出口** — DNS 配置读值虚拟化无法约束 UDP 源地址；DNS 伪装正常也不能阻止 WebRTC。
3. **应用自己拼启动参数不可靠** — Chromium 系支持 `--force-webrtc-ip-handling-policy=disable_non_proxied_udp`，WebView2 支持 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`，但用户要逐个应用手填；子进程（utility / GPU / WebView2 browser process）还可能丢参。
4. **Packaged / PostActivation 错过启动参数** — AUMID 激活后根进程已在跑，Chromium 启动 flag 可能已经错过，仅靠 Browser Policy 不够，需要更强的兜底。
5. **泄漏常发生在异常路径** — 代理短暂断开、TUN 切换、IPv6 绕行时浏览器 fallback 到 direct UDP，真实出口暴露；正常路径测不出。

用户要的不是「把 WebRTC 检测结果伪装掉」，而是：**把 Aura Session 这棵进程树的 WebRTC / direct UDP 网络路径真正约束住**，且完全不影响同程序在 Session 外的实例。

## Solution

把 WebRTC 防泄漏做成 **Browser / Network Guard** 独立能力（Privacy Profile 的一个字段），**不塞进现有 Geo/Locale Hook**。三层递进：

```
Aura Session
│
├─ ① Browser Policy
│     Chromium/WebView2：disable_non_proxied_udp
│
├─ ② Runtime Child Guard
│     CreateProcess 识别 Browser Engine → 自动继承策略
│
└─ ③ Network Guard
      即使浏览器不听，Session 内 direct UDP 也不放行
```

- **① Browser Policy（阶段 1，性价比最高）**：Profile 增加 `BrowserPrivacyProfile`。启动 Win32 Chromium/Electron 根进程时追加 Chromium switch；WebView2 场景写 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 进 Environment Block / 子进程继承。默认策略倾向 `DisableNonProxiedUdp`（产品级 Privacy Profile）。
- **② Runtime Child Guard（阶段 2）**：扩展已验证的 `hooks_process` 子进程传播缝。对明确分类的 Browser Engine 子进程（Chromium / Edge / WebView2 / Electron）注入/确认 WebRTC policy 与 WebView2 环境变量；冲突时按 Profile 策略显式处理，不盲目重复 append。
- **③ Network Guard（阶段 3）**：进程树内 UDP 出站约束。Balanced = 仅 Browser Policy；Strict = Policy + direct UDP 限制。Session 级精准（按 PID ∈ EnvironmentSession），不是全局防火墙。先评估用户态 WFP Filter（`ALE_APP_ID` 粒度不够，仅作过渡验证）；真正「只限本进程树」需要小型 WFP Callout Driver 读 `FWPS_METADATA_FIELD_PROCESS_ID`，对齐 Session Registry — **该驱动不进阶段 1/2**。

对用户可见的产品承诺：选 Application + Profile → Run → 进程树既看到一致 Environment View，WebRTC 也不会从另一条 UDP 路径漏出真实出口；Host 全局配置不改；策略未生效不得静默当成成功。

## User Stories

1. As a user, I want a Privacy Profile field for WebRTC policy on my Environment Profile, so that browser network-path constraints travel with the same profile as Locale/Timezone/DNS.
2. As a user, I want the default WebRTC policy to prevent non-proxied UDP, so that I do not leak a real local/public IP just because I forgot a flag.
3. As a user, I want Host WebRTC behavior unchanged when I run with WebRTC policy `Host`, so that debugging and legitimate peer apps still work.
4. As a user, I want `PublicInterfaceOnly` to stop exposing private/local interface addresses while still allowing normal public UDP, so that I can choose a lighter mode when appropriate.
5. As a user, I want `ProxyOnly` / `DisableNonProxiedUdp` to make Chromium-family browsers use UDP only through a UDP-capable proxy, so that ICE candidates do not come from my real NIC.
6. As a user, I want `Strict` to also block direct UDP from the Session even if the browser ignores flags, so that Packaged Chromium and hostile misconfig cannot fall back to a real path.
7. As a user, I want WebRTC policy applied when I launch a Chromium/Electron app under Aura without remembering command-line switches, so that protection is default-on for the session.
8. As a user, I want WebView2 child browser processes to receive `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` with the WebRTC policy, so that agents embedding WebView2 inherit the same constraint.
9. As a user, I want browser utility/GPU/renderer children to keep the same WebRTC policy as their parent, so that multi-process Chromium cannot restart a leaking helper.
10. As a user, I want a Packaged WindowsApps Chromium/WebView2 app to still be constrained even if activation already happened, so that PostActivation race cannot become a permanent leak.
11. As a user, I want Aura to classify browser engines explicitly (Chromium / Edge / WebView2 / Electron), so that random `chrome`-named binaries are not mis-armed and real browsers are not missed.
12. As a user, I want conflict handling when the target command line already contains a WebRTC IP policy flag, so that Profile policy wins deterministically and flags are not duplicated.
13. As a user, I want `envbox-browser-probe` to report ICE candidates under Host vs Aura policies, so that I can verify behavior myself without third-party leak sites.
14. As a user, I want acceptance to include IPv4 and IPv6, so that a real IPv6 candidate cannot bypass a v4-only check.
15. As a user, I want the proxy-down / TUN-down case tested, so that fallback to direct UDP is treated as failure, not as a rare footnote.
16. As a user, I want Strict mode to allow only explicitly authorized UDP egress (if any), so that WebRTC turns into TCP/proxy fallback instead of silent direct UDP.
17. As a user, I want HTTP/3 degradation under Strict to be accepted and documented, so that I am not surprised when the browser falls back to TCP.
18. As a user, I want STUN/TURN port blocklists avoided, so that protection does not depend on well-known ports that can move.
19. As a user, I want Host Chrome I launch myself outside Aura to be completely unaffected by another Session’s Strict policy, so that process-scoped guarantees hold.
20. As a user, I want session stop to remove any Session-scoped network constraints, so that Host networking returns to a clean state.
21. As a developer, I want `BrowserPrivacyProfile` / `WebRtcPolicy` as domain types on Profile (not scattered booleans), so that CLI/GUI/Runtime share one vocabulary.
22. As a developer, I want browser policy injection on the existing CreateProcess child seam, so that I do not add a second process-create hook stack.
23. As a developer, I want a pure BrowserEngine → policy decision table, so that classification and flag/env derivation are unit-testable without a real browser.
24. As a developer, I want Browser Policy and Network Guard as separate layers, so that Balanced mode ships without a driver and Strict can land later.
25. As a developer, I want Network Guard, if implemented, to consult Session PID membership (Session Registry / Process Tracker), so that only the Aura process tree is filtered.
26. As a developer, I want Audit events for browser policy applied / conflict resolved / network deny, so that leaks and mis-applies are diagnosable.
27. As a user, I want GUI to show WebRTC Privacy next to DNS/locale on the Profile card, so that I can see the active network-path policy at a glance.
28. As a user, I want Startup Fail Policy when Strict cannot be enforced (e.g. required guard unavailable), so that I never believe Strict is on when it is not.
29. As a developer, I want `envbox-browser-probe` as a first-class acceptance tool beside `envbox-probe`, so that environment view and network path have separate oracles.
30. As a user, I want Host system firewall/proxy settings unchanged by Aura, so that Browser/Network Guard stays Host-transparent.

## Implementation Decisions

- **独立模块，不进 Geo/Locale Hook。** WebRTC 属于 Browser / Network Guard。Geo/Locale/DNS Runtime Hook 继续只负责 Environment View；两边可以共享 Profile / Session / IPC，不共享 hook 实现。
- **领域模型。** `Profile` 增加 `BrowserPrivacyProfile { webrtc: WebRtcPolicy }`。`WebRtcPolicy`：`Host` / `PublicInterfaceOnly` / `ProxyOnly`（= Chromium `disable_non_proxied_udp`）/ `Strict`（= ProxyOnly + Network Guard）。默认 Privacy Profile 建议 `ProxyOnly`；`Strict` 显式选择。
- **Browser Policy 通道（阶段 1）。** Win32 Chromium/Electron：Activation/launch 路径在根进程命令行追加 `--force-webrtc-ip-handling-policy=...`（与 Chromium 现行 switch 对齐）。WebView2：写入 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`（Environment Block + 子进程 `UpsertProfileKeys` 继承）。不在此层改 DNS/Locale。
- **Browser Engine 分类（阶段 2）。** 显式枚举 `Chromium` / `Edge` / `WebView2` / `Electron` / `Unknown`。分类依据打包/路径/镜像名的明确规则表，禁止「命令行含 chrome 就加」。`Unknown` 不改命令行，但可在 Strict Network Guard 下仍受 UDP 约束。
- **CreateProcess 缝复用。** 在现有 Runtime 子进程传播（CreateProcess* → suspend → register → inject → resume）上增加 Browser Child Guard：识别 Browser Engine 后应用/确认 WebRTC policy 与 WebView2 环境变量，再创建子进程。不新建第二套 process hook。
- **冲突语义。** 目标命令行已含 `--force-webrtc-ip-handling-policy` 或已有 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 时：以 `BrowserPrivacyProfile` 为准改写/规范化，避免重复 append；`Host` 策略不覆盖用户显式参数。决策表用纯函数落地并单测。
- **Packaged / PostActivation。** AUMID 激活后可能错过根进程启动 flag。阶段 1 对 Packaged 仅保证 WebView2 环境与子进程策略；根 Chromium 已起来的场景 **必须** 依赖阶段 3 Network Guard 才能满足 Strict。产品文案区分 Balanced / Strict 保证级别（类似 IsolationGuarantee）。
- **Network Guard（阶段 3）。** 语义：Session 内 Browser/相关进程的 UDP 出站只允许 Profile 授权出口；无 UDP-capable proxy 则 direct UDP deny，允许应用回退 TCP/proxy。禁止按 STUN/TURN 端口封禁。用户态 WFP Filter（`ALE_APP_ID`）仅作开发验证，因分不清「Aura Chrome」与「用户 Chrome」。Session 级精准需要 WFP Callout Driver 用 `FWPS_METADATA_FIELD_PROCESS_ID` 查 Session Registry — **驱动单独里程碑，受 AGENTS.md「不提前做 WFP Driver」约束，不进本 spec 阶段 1/2 交付。**
- **Fail 语义。** 单条 Browser Policy hook 失败 → Fail Open（回退原 CreateProcess / 不改参数），但 Audit 记录。`Strict` 承诺若无法执行（Guard 不可用）→ Startup Fail Policy，禁止静默降级为 Balanced。Balanced 下 Browser Policy 未识别到浏览器引擎不视为失败。
- **Profile 持久化与 IPC。** `BrowserPrivacyProfile` 进 serde Profile；IPC PROFILE DTO 扩展 `webrtc` 字段（与 V0.3 RuntimeProfile 通道一致）。Packaged 无 Environment Block 时走 Broker；WebView2 变量仅 Win32 Environment Block / 子进程回退有意义。
- **GUI / CLI。** Profile 编辑增加 WebRTC Privacy 选择；应用详情展示策略与保证级别。CLI `profile add` 增加对应参数。不改 `envbox run` 对非浏览器目标的语义。
- **命名。** 产品能力名 Browser / Network Guard；配置名 `BrowserPrivacyProfile` / `WebRtcPolicy`。实现沿用 `envbox-*` crate / Runtime 命名，不引入第二品牌前缀。

## Testing Decisions

- **好测试只看外部行为**：在给定 Profile 策略下，probe 观察到的 ICE candidate 类型 / 是否出现非代理 UDP / 是否出现真实公网 IP；不测 Chromium 内部、不测 WFP 字段布局、不测命令行字符串内部实现细节（冲突表除外，因其即外部契约）。
- **主测试缝（优先，理想接近一条）**：`envbox run --profile <p> -- tools/envbox-browser-probe`（新工具）输出 JSON/文本报告：ICE candidates、transport、本地/候选 IP 分类（private / public / proxy）、policy 生效标志。既有 `envbox run` + Probe 矩阵是先例。
- **次缝（纯规则，无浏览器）**：BrowserEngine 分类与 policy 决策表（已知 switch / WebView2 变量 / 冲突改写）的单元测试，风格对齐 `capability` / `evaluate_injection_support` 策略表测试。
- **场景矩阵（probe 为准，不依赖 IPPure 第三方页）**：
  - Host vs Aura Balanced / ProxyOnly / Strict
  - 不出现 local/private IP（Balanced+）
  - 不出现 non-proxy UDP candidate（ProxyOnly+）
  - Strict 无任何 direct UDP candidate
  - IPv6 不允许真实地址绕出
  - **Proxy down / TUN down：不得 fallback 真实直连**
  - Child WebView2 与 Parent Profile 一致
  - WindowsApps / Packaged WebView2 与 Profile 一致（Strict 才承诺根 Chromium）
  - IPv4 only / IPv6 enabled；TUN on/off
- **回归**：现有 `cargo test --workspace`、`envbox-probe` 环境视图矩阵、Host 配置不变检查必须保持全绿；非浏览器应用的 CreateProcess 行为不得被 Browser Child Guard 改变。
- **审计**：policy applied / conflict / network deny 事件可断言（对齐 Audit JSONL 测试风格）。
- **Prior art**：`.scratch/envbox-v02` DNS routing 验收矩阵、`cli_run`/`cli_boundary` 子进程继承测试、V0.3 packaged smoke（`chatgpt_packaged_aumid_starts` 风格的 machine-conditional）。

## Out of Scope

- 字体虚拟化、硬件指纹、磁盘/BIOS/GPU 序列号、MAC 伪造、反检测/隐藏 EnvBox。
- 「把 WebRTC 检测页结果改好看」——只约束真实网络路径，不伪造探测结果。
- 按 STUN/TURN 端口（3478/19302 等）封禁。
- 全局系统代理、全局 VPN、透明流量转发、DoH/LSP/端口 53 劫持（DNS 非目标保持不变）。
- **阶段 1/2 不做 WFP Callout Driver / 内核驱动**；用户态 WFP 仅开发期过渡，不作为产品承诺。
- AppContainer / Protected Process 的注入与 WebRTC 保证（仍 Fail Closed）。
- 修改 Host 防火墙规则、Host 系统时区/区域、Host 代理设置。
- 非 Chromium 系浏览器引擎的完整策略语义（Firefox 等：`Unknown`，仅 Strict 网络层可约束）。

## Further Notes

- **这是对历史「明确不做 WebRTC」的有意扩围**，且刻意拆成 Browser / Network Guard，而不是塞进 Geo/Locale Hook。V0.1–V0.3 文档中的 non-goal 保留原意（当时不做 hook/WebRTC）；本 feature 扩围后应在 isolation-tiers / CONTEXT 增加 Browser Privacy 词汇与分级承诺。
- **哲学对齐**：与 EnvBox「Environment-consistent」一致 — 约束 Session 真实路径，而不是伪造检测面。和字体相比优先做 WebRTC：字体多是特征面，WebRTC 是另一条真实网络出口。
- **阶段 1 最划算**：BrowserPrivacyProfile + Chromium/WebView2 policy + envbox-browser-probe。阶段 2 接现有 CreateProcess 子进程传播。阶段 3 再谈 Network Guard；若产品最终要求「只限本进程树、完全不影响同程序其他实例」，才启动小型 Callout Driver 里程碑。
- **Guarantee 分级建议**与 IsolationGuarantee 并列文档化：`BrowserPolicyOnly`（Balanced）/ `NetworkEnforced`（Strict）。Packaged 根进程 Strict 依赖 Network Guard。
- 测试缝若需调整，优先保持 **一条主缝（run + browser-probe）**，避免把 Chromium/WebView2 细节测进单测。

## Proposed tickets (planning only)

| Phase | Ticket | 交付 |
|-------|--------|------|
| 1 | 51 | `BrowserPrivacyProfile` / `WebRtcPolicy` 模型 + IPC/Profile 字段 |
| 1 | 52 | Chromium/WebView2 Browser Policy（根进程 + env） |
| 1 | 53 | `envbox-browser-probe` + Host/Balanced/ProxyOnly 验收矩阵 |
| 2 | 54 | Browser Engine 分类 + CreateProcess Child Guard |
| 2 | 55 | 冲突语义 + Audit + GUI 展示 |
| 3 | 56 | Network Guard（用户态验证 / Session PID 语义设计） |
| 3+ | 57 | WFP Callout Driver 里程碑（单独评估，受 AGENTS.md 约束） |

依赖：51 → 52 → 53 → 54 → 55 → 56；57 独立。
