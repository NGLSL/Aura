Parent: .scratch/envbox-packaged-v1/spec.md

# 39: Packaged Activation Backend（AUMID）

**What to build:** `PackagedActivationBackend` — 仅经 `IApplicationActivationManager::ActivateApplication` 启动并返回 PID；禁止直接跑 WindowsApps exe 作为 root。

**Blocked by:** 38

**Status:** ready-for-agent

- [ ] `activate(target) -> ActivatedTarget`（pid、package_identity、running、capabilities）
- [ ] 不使用 Environment Block；不改 package lifecycle
- [ ] 验收：启动一个真实 mediumIL packaged Win32 并拿到 PID
- [ ] AppContainer 目标不进入成功路径

## Comments
