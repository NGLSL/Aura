# Isolation Tiers & Limits (V0.3)

产品承诺不变：**一个进程树看到一套 Environment View**。不是沙箱，不是 VM。

## Tier 模型（文档概念）

| Tier | 目标 | 注入时机 | IsolationGuarantee | 支持 |
|------|------|----------|--------------------|------|
| Tier 1 | 原生 Win32 / Command | PreExecution（挂起注入） | FullPreExecution | ✅ |
| Tier 2 | Packaged Win32 mediumIL | PostActivation（AUMID 后注入） | PostActivation | ✅（有 early-start race） |
| Tier 3 | AppContainer / Protected Process | — | — | ❌ Fail Closed |

## early-start race（已知限制）

Packaged root 经 `ActivateApplication` 启动后才能注入 Runtime。在「激活后、注入前」的窗口内，目标可能已经读到 Host 的 Geo/Locale/Timezone 并缓存。

- 子进程仍可 pre-execution 注入，不受此限制。
- 产品上把该窗口建模为 `IsolationGuarantee::PostActivation`，不是 bug。
- 启动即读环境并永久缓存的 Store 应用，可能显示 Host 值。

## Broker / ENVBOX 配置通道

| 优先级 | 通道 | 适用 | 说明 |
|--------|------|------|------|
| 1 | Runtime IPC Bootstrap（Broker PROFILE DTO） | 全部 | 含 packaged root（无 Environment Block） |
| 2 | ENVBOX_* 结构化值回退 | Win32 | Host 写入 Environment Block；**C++ 不解析 profiles.toml** |
| 3 | 皆失败 | — | Startup Fail Policy，禁止静默无虚拟化 |

## Fail Open vs Fail Closed

- **Fail Open**：单个 Hook 失败 → 回退原 Windows API（兼容优先）。
- **Fail Closed**：无法激活 / 无法注入 / 无 Profile / Capability Unsupported → 启动失败，不静默降级。

## Capability 规则（无 bypass）

| 目标 | 结果 |
|------|------|
| Win32 x64 / x86 | Supported |
| Packaged Win32 mediumIL、无阻断 mitigation | Supported（PostActivation） |
| AppContainer | Unsupported |
| Protected Process | Unsupported |
| MicrosoftSignedOnly / StoreSignedOnly / Unknown mitigation | Unsupported |
| 无法打开/查询进程 | Unsupported |

## Browser / Network Guard（WebRTC Privacy）

对历史「明确不做 WebRTC」的**有意扩围**。独立于 Geo/Locale Hook，属于 Browser / Network Guard。

| WebRtcPolicy | Browser Guarantee | 执行层 | 说明 |
|--------------|-------------------|--------|------|
| `Host` | — | 不改浏览器 | 调试与合法 P2P 可用 |
| `PublicInterfaceOnly` | PolicyOnly | Chromium `default_public_interface_only` | 隐藏私网/本地接口 |
| `ProxyOnly` | PolicyOnly | Chromium `disable_non_proxied_udp` | UDP 仅经代理；默认推荐 |
| `Strict` | NetworkEnforced | ProxyOnly + Network Guard | 进程树 direct UDP deny；**无 Guard 则 Startup Fail** |

### Packaged / PostActivation 限制

AUMID 激活后根 Chromium 可能已错过启动 flag。阶段 1 对 Packaged 仅保证 WebView2 环境与子进程策略；根进程 Strict 承诺依赖 Network Guard。

### Network Guard 语义（阶段 3）

- Session 进程树 UDP 出站仅允许 Profile 授权出口；无 UDP-capable proxy 则 direct UDP deny。
- **禁止**按 STUN/TURN 端口封禁。
- Session 归属查 Session Registry / Process Tracker（PID）；`ALE_APP_ID` 仅作开发验证，不是产品承诺（分不清 Aura Chrome 与用户 Chrome）。
- 真正「只限本进程树」的内核路径需 WFP Callout Driver（`FWPS_METADATA_FIELD_PROCESS_ID`）——**单独里程碑，不进阶段 1/2**。
- Strict 无法执行 → Startup Fail Policy，禁止静默降级为 Balanced。
- HTTP/3 在 Strict 下回退 TCP 为可接受行为。
- stop 后移除 Session 约束；Host 防火墙/全局配置不变。

### Browser Guarantee 分级

| 名称 | 含义 |
|------|------|
| `BrowserPolicyOnly` | 仅 Browser Policy 产物（Balanced / ProxyOnly） |
| `NetworkEnforced` | Policy + Network Guard（Strict） |

## 明确不做

Sandbox / VM / 驱动级隔离 / 反检测 / 字体虚拟化 / 完整 Registry Sandbox。
（WebRTC 已扩围为 Browser / Network Guard，见上节；仍不做「把检测页结果改好看」。）
