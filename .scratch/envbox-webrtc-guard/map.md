# envbox-webrtc-guard map

Spec: [spec.md](./spec.md) · Status: ready-for-agent

产品定位：Browser / Network Guard（WebRTC Privacy），独立于 Geo/Locale Hook。

| Phase | Ticket | 内容 | Blocked by | Status |
|-------|--------|------|------------|--------|
| 1 | [51](./issues/51-browser-privacy-profile-model.md) | BrowserPrivacyProfile / WebRtcPolicy 模型与 Profile/IPC | — | ready-for-agent |
| 1 | [52](./issues/52-browser-policy-chromium-webview2.md) | Chromium / WebView2 Browser Policy | 51 | ready-for-agent |
| 1 | [53](./issues/53-envbox-browser-probe.md) | envbox-browser-probe + 验收矩阵 | 52 | ready-for-agent |
| 2 | [54](./issues/54-browser-child-guard.md) | Browser Engine 分类 + CreateProcess Child Guard | 53 | ready-for-agent |
| 2 | [55](./issues/55-conflict-audit-gui.md) | 冲突语义 + Audit + GUI | 54 | ready-for-agent |
| 3 | [56](./issues/56-network-guard-strict.md) | Network Guard（Session 级 UDP） | 55 | ready-for-agent |
| 3+ | [57](./issues/57-wfp-callout-driver-milestone.md) | WFP Callout Driver（单独评估） | 56 | ready-for-agent |

依赖主链：51 → 52 → 53 → 54 → 55 → 56；57 独立评估。

## 测试缝

1. **主缝**：`envbox run` + `tools/envbox-browser-probe`
2. **次缝**：BrowserEngine → policy 纯决策表单测

## 范围提醒

- 阶段 1/2 无内核驱动；`Strict` 全承诺依赖阶段 3 Network Guard（Runtime WinSock）。
- 不做字体/指纹/反检测/端口封禁；Host 配置不变。
- 相对 V0.x「不做 WebRTC」为有意扩围，交付时同步 `docs/isolation-tiers.md` 与 `docs/CONTEXT.md`。

## 实现备注（交付中）

- 51/52：`browser_policy.rs` 决策表 + IPC `webrtc=` + Environment Block + 根进程 argv switch + CLI `--webrtc`。
- 56：`hooks_network.cpp`（Strict 下 deny 非 loopback UDP）+ `NetworkGuardCapability` Startup Fail 门禁。
- 57 评估结论：优先 Runtime 进程树 Hook；WFP Driver 暂缓（见 issue 57）。
