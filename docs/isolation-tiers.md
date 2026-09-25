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

## 明确不做

Sandbox / VM / 驱动级隔离 / 反检测 / 字体虚拟化 / WebRTC Hook / 完整 Registry Sandbox。
