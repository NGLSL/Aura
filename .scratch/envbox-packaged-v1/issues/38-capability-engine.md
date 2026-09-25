Parent: .scratch/envbox-packaged-v1/spec.md

# 38: Capability Engine + Package discovery

**What to build:** 目标能力探测与已安装 package 元数据；用 TargetCapabilities 选后端，禁止路径型 `if WindowsApps`。

**Blocked by:** 37

**Status:** ready-for-agent

- [ ] `TargetCapabilities`：can_suspend / can_inject_runtime / can_create_environment_block / can_assign_job / can_track_children
- [ ] 进程级 Probe：TokenIsAppContainer、Integrity、SignaturePolicy、DynamicCodePolicy、ImageLoadPolicy
- [ ] 规则：AppContainer / MicrosoftSignedOnly / StoreSignedOnly / 无法查询 → Unsupported；mediumIL 无阻断 → Supported
- [ ] Package discovery：DisplayName、AUMID、PackageFullName、PackageFamilyName、RuntimeBehavior、TrustLevel
- [ ] 单元表测 capability 规则（无需真商店应用）

## Comments
