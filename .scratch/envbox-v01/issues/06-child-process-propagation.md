Parent: .scratch/envbox-v01/spec.md

# 06: 子进程传播（Process Tree Instance）

**What to build:** 目标进程树内创建的 `cmd`/`node`/`powershell`/`git`/`python` 等子进程默认继承同一 Environment Profile；用户从任务栏另开的进程完全不受影响。隔离单位是 Process Tree Instance，不是可执行文件名。

**Blocked by:** 05 Minimal Profile — 4 个核心 API

**Status:** ready-for-agent

- [x] Hook `CreateProcessW`/`CreateProcessA`：强制 `CREATE_SUSPENDED` → 注入 Runtime → 保证 Profile ID 继承 → 仅当 `caller_requested_suspended=false` 时 Resume
- [x] 调用方已要求 suspend 时注入后保持 Suspended 并返回调用方
- [x] `envbox-probe --spawn-child` 父子虚拟化字段一致
- [ ] 矩阵：`cmd`→`node` 树（及 git/powershell/python）继承；无关进程不受影响
- [x] 同树共享同一 `ENVBOX_INSTANCE_ID` / Profile 视图

## Comments

- Ticket 06 delivered. `CreateProcessW/A` hooks: force `CREATE_SUSPENDED` → `DetourUpdateProcessWithDll` → Resume only if `caller_requested_suspended=false`. Custom env blocks are converted to Unicode (`CREATE_UNICODE_ENVIRONMENT`) and upsert `ENVBOX_*` + `ENVBOX_CONFIG_ROOT`/`ENVBOX_RUNTIME_DLL`. Inject/Resume failure terminates the child and returns FALSE with saved `GetLastError` (no silent unvirtualized child). `ENVBOX_INHERIT_CHILDREN=0` → plain create (Application.inherit_children channel).
- Review fixes: Startup Fail Policy when Runtime DLL path missing; Unicode env overlay + ANSI/W parse; SetLastError on inject/Resume fail; Profile load falls back to `%LOCALAPPDATA%\EnvBox` and requires full field set (no empty success).
- Evidence: `cargo test --workspace` 56 passed — `run_probe_spawn_child_inherits_profile` (same instance id + 4 fields), `run_cmd_to_probe_child_sees_profile`, `run_no_inherit_children_leaves_child_unhooked`, `run_powershell_child_sees_profile_when_available`.
- Deferred to ticket 12 matrix: `node`/`git`/`python` trees. `caller_requested_suspended` is implemented (Resume skipped) but not covered by an automated test (needs a suspended-create harness). Application.inherit_children is wired via `ENVBOX_INHERIT_CHILDREN`; `envbox run` exposes `--no-inherit-children` for the same channel.

