Parent: .scratch/envbox-v01/spec.md

# 04: Detours 注入烟测

**What to build:** 用户通过 EnvBox 启动 Probe 时，`envbox-runtime64.dll` 在应用逻辑前载入并打出载入标记；注入失败则整次启动失败，不降级为普通启动。

**Blocked by:** 03 envbox run 启动通路（无 Hook）

**Status:** done

- [x] `CREATE_SUSPENDED` + `DetourCreateProcessWithDllEx` 注入 Runtime，再 Resume 主线程
- [x] DllMain：`DetourIsHelperProcess` → `DetourRestoreAfterWith` → Load Profile →（暂不）Install Hooks
- [x] Probe 输出可见 “EnvBox Runtime Loaded”（或等价稳定标记）
- [x] Runtime DLL 缺失/注入失败 → Startup Fail Policy 拒绝启动并显示原因
- [x] Host 系统配置仍无任何变化

## Comments

- Ticket 04 delivered. `DetourCreateProcessWithDllExW` + `CREATE_SUSPENDED` → Job assign → Resume. DllMain: `DetourIsHelperProcess` → `DetourRestoreAfterWith` → Load Profile (IDs + `profiles.toml` presence) → hooks deferred. Probe prints `EnvBox Runtime Loaded` only when the runtime module is mapped. Missing/corrupt Runtime DLL fails closed. Host probe GEO/LOCALE/LANGUAGE/TIMEZONE/DNS unchanged across Run.
- Review fixes: save `GetLastError` on encode/`TerminateProcess`; Job-assign failure must kill the suspended root (no leak); marker requires `GetModuleHandle`; stale docs fixed; arch-preferring DLL resolve.
- Deferred: full immutable `RuntimeProfile` field parse (ticket 05); `caller_requested_suspended` on child CreateProcess (ticket 06). Root `envbox run` always resumes after inject.
- Evidence: `cargo test --workspace` 49 passed (`run_probe_prints_runtime_loaded_marker`, `run_missing_runtime_dll_fails_closed`, `run_corrupt_runtime_dll_fails_closed`, `run_leaves_host_probe_snapshot_unchanged`).

