Parent: .scratch/envbox-webrtc-guard/spec.md

# 53: envbox-browser-probe 与验收矩阵（Phase 1）

**What to build:** 独立网络路径探针，作为 WebRTC Privacy 验收基准（不依赖 IPPure 等第三方页）。

**Blocked by:** 52

**Status:** resolved

- [x] `tools/envbox-browser-probe`：最小 ICE/STUN candidate 采集，输出 JSON/文本（candidate、transport、local/public/proxy 分类、policy 标记）
- [x] 能在 `envbox run --profile ...` 下作为目标运行，并对比 Host vs Aura
- [x] 场景矩阵（至少）：Host / Balanced / ProxyOnly；IPv4+IPv6；记录 candidate 是否含真实公网 IP / non-proxy UDP
- [x] Proxy down / TUN down：断言 **不得** fallback 真实直连（失败即验收失败）— `--assert` + Strict 路径
- [x] 无真浏览器依赖的可自动化部分进 `cargo test`/脚本；需真 Chromium/WebView2 的标 machine-conditional
- [x] 与 `envbox-probe`（环境视图）职责分离：本工具只报告网络/WebRTC 路径

## Comments
