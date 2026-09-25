Parent: .scratch/envbox-webrtc-guard/spec.md

# 55: 冲突语义完善 + Audit + GUI 展示（Phase 2）

**What to build:** 策略冲突可诊断、可观测；Profile 卡片可见 WebRTC Privacy 与保证级别。

**Blocked by:** 54

**Status:** resolved

- [x] 冲突规则完备（重复 switch、WebView2 变量已存在、多策略叠加）并有表驱动测试
- [x] Audit 事件稳定 schema（applied / conflict / ignored-unknown）— `PolicyApply::audit_api`
- [x] GUI Profile 编辑：WebRTC Privacy 选择（Host / PublicInterfaceOnly / ProxyOnly / Strict）
- [x] GUI 应用/实例展示当前 policy 与 Browser Guarantee（PolicyOnly vs NetworkEnforced 预留）
- [x] CLI `profile list` 显示该字段
- [x] 非浏览器应用 UI/行为无回归

## Comments
