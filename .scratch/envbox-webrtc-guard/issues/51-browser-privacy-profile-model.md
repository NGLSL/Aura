Parent: .scratch/envbox-webrtc-guard/spec.md

# 51: BrowserPrivacyProfile / WebRtcPolicy 模型与 Profile 字段（Phase 1）

**What to build:** 在领域模型与 Profile 持久化中落地 WebRTC Privacy 配置，作为 Browser / Network Guard 的唯一配置源。

**Blocked by:** — 

**Status:** resolved

- [x] `WebRtcPolicy`：`Host` / `PublicInterfaceOnly` / `ProxyOnly` / `Strict`（域类型，非散落 bool）
- [x] `BrowserPrivacyProfile { webrtc: WebRtcPolicy }` 挂到 `EnvironmentProfile`
- [x] serde 往返（TOML/JSON 存储）；缺省语义明确（默认 `Host` 兼容；产品推荐 `ProxyOnly`）
- [x] RuntimeProfile / IPC PROFILE DTO 扩展 `webrtc` 字段（与 V0.3 通道一致，C++ 不解析业务枚举语义）
- [x] CLI `profile add --webrtc` / `profile list` 显示该字段（GUI 见 55）
- [x] 单测：校验、默认值、序列化往返、非法值拒绝
- [x] 文档：CONTEXT 增加 Browser Privacy / WebRtcPolicy 词汇

## Comments
