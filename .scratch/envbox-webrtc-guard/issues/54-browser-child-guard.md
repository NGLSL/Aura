Parent: .scratch/envbox-webrtc-guard/spec.md

# 54: Browser Engine 分类 + CreateProcess Child Guard（Phase 2）

**What to build:** 在现有子进程传播缝上识别 Browser Engine，子进程继承同一 WebRTC Privacy。

**Blocked by:** 53

**Status:** resolved

- [x] `BrowserEngine`：`Chromium` / `Edge` / `WebView2` / `Electron` / `Unknown`（明确规则，禁止裸字符串 `chrome` 匹配）
- [x] CreateProcess* hook：识别 Browser 子进程后应用/确认 policy（含 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`）
- [x] utility/GPU/renderer/WebView2 browser process 与 Parent Profile 一致（多进程不丢策略）
- [x] `inherit_children=false` 语义不被破坏（按 Application 开关）
- [x] Packaged 场景：能处理则处理子进程；根进程已错过启动参数的限制写入 Guarantee，不伪称 Balanced 能救
- [x] 决策表单测 + 多级子进程注入回归（对齐 t33 风格）— Rust `plan_child_policy` 表测 + C++ 分类

## Comments
