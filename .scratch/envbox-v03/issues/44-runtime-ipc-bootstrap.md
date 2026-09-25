Parent: .scratch/envbox-v03/spec.md

# 44: Runtime IPC 握手接入 Broker（Phase 2b）

**What to build:** Runtime DLL 经 Named Pipe 向 Broker 按 PID 取 RuntimeProfile；子进程创建时通知 Broker；保留 ENVBOX_* 回退，避免一次性破坏 Win32。

**Blocked by:** 43

**Status:** resolved

- [x] DLL init：HELLO(pid) → GET_PROFILE → PROFILE 填入 RuntimeProfile；失败可回退 ENVBOX_* 路径
- [x] PROFILE 映射完整：locale / ui_language / region / timezone / dns / registry whitelist / inherit_children / audit / ids
- [x] Process hooks 在创建子进程时上报 PROCESS_CREATED（parent/child PID），退出可上报 PROCESS_EXITED
- [x] Broker 可用 + ENVBOX 可用：优先 Broker；Broker 不可用 + ENVBOX 可用：Win32 仍虚拟化
- [x] Broker 不可用 + 无 ENVBOX：Startup Fail Policy，禁止静默无虚拟化
- [x] Win32 金路径 Probe 矩阵保持全绿（回退不回归）

## Comments

### 2026-09-27 done

C++ `hooks_process.cpp` 在注入成功后 `EnvBoxIpcNotifyProcessCreated`；`UpsertProfileKeys` 传递 ENVBOX_* 值与 IPC pipe。加载顺序：IPC PROFILE → ENVBOX_* 值 → Fail Closed。`cargo test` run+Probe 矩阵全绿。
