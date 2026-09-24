Parent: .scratch/envbox-v01/spec.md

# 04: Detours 注入烟测

**What to build:** 用户通过 EnvBox 启动 Probe 时，`envbox-runtime64.dll` 在应用逻辑前载入并打出载入标记；注入失败则整次启动失败，不降级为普通启动。

**Blocked by:** 03 envbox run 启动通路（无 Hook）

**Status:** ready-for-agent

- [ ] `CREATE_SUSPENDED` + `DetourCreateProcessWithDllEx` 注入 Runtime，再 Resume 主线程
- [ ] DllMain：`DetourIsHelperProcess` → `DetourRestoreAfterWith` → Load Profile →（暂不）Install Hooks
- [ ] Probe 输出可见 “EnvBox Runtime Loaded”（或等价稳定标记）
- [ ] Runtime DLL 缺失/注入失败 → Startup Fail Policy 拒绝启动并显示原因
- [ ] Host 系统配置仍无任何变化
