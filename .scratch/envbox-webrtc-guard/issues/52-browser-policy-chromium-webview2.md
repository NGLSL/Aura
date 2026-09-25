Parent: .scratch/envbox-webrtc-guard/spec.md

# 52: Chromium / WebView2 Browser Policy（Phase 1）

**What to build:** 在根进程启动与 Environment Block 通道上应用 WebRTC policy，不改 Geo/Locale Hook。

**Blocked by:** 51

**Status:** resolved

- [x] Win32 Chromium/Electron 根启动：按 `WebRtcPolicy` 追加/规范化 `--force-webrtc-ip-handling-policy=...`（`ProxyOnly` → `disable_non_proxied_udp`；`PublicInterfaceOnly` 对应 Chromium 策略；`Host` 不追加）
- [x] WebView2：写入 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`（Environment Block + 子进程 UpsertProfileKeys 继承）
- [x] `Unknown` 引擎不改命令行（Balanced 可接受）
- [x] 冲突：已有 policy switch / 已有 WEBVIEW2 变量时以 Profile 为准规范化，不重复 append；`Host` 不覆盖用户显式参数
- [x] 非浏览器目标 CreateProcess 行为不变（回归）
- [ ] Audit：policy applied / conflict-resolved 事件（对齐既有 JSONL）— 并入 55
- [x] 纯决策表单测：policy → flags/env 映射与冲突规则

## Comments
