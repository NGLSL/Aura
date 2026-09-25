Parent: .scratch/envbox-packaged-v1/spec.md

# 37: EnvironmentSession + Activation/Attach 拆分

**What to build:** 控制面从 `launch()` 一体拆为 Session → ActivationBackend → RuntimeAttacher；Win32 行为锁死不回归。

**Blocked by:** （无；先锁 Win32 金路径）

**Status:** ready-for-agent

- [ ] `docs/CONTEXT.md`：EnvironmentSession / ActivationBackend / AttachStrategy / TargetCapabilities / IsolationGuarantee
- [ ] `EnvironmentSession`：id、target、profile_id、root_processes、processes、package_identity?、state、isolation_guarantee
- [ ] `ActivationBackend` / `RuntimeAttacher` 接口；`Win32ActivationBackend` 仍 `CreateProcess(SUSPENDED)`
- [ ] `LaunchTarget` 扩展 Packaged 变体（serde 兼容旧 `executable`/`command`）
- [ ] 现有 `envbox run` + Probe 验收矩阵保持全绿（PreExecution 语义不变）

## Comments

### 2026-09-26 design

目标态是 Environment Session Runtime，不是 DLL Injector。先拆控制面、锁 Win32，再上 Packaged。
