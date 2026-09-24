Parent: .scratch/envbox-v01/spec.md

# 03: envbox run 启动通路（无 Hook）

**What to build:** 用户执行 `envbox run --profile <id> <command>` 能按 Application/Profile 语义启动目标进程：命令解析、独立 Environment Block、Arguments/CWD、Job Object 跟踪；失败原因可见，且绝不静默按 Host 环境“假成功”。

**Blocked by:** 02 Profile & Application 持久化 + 校验 + CLI List

**Status:** done

- [x] Command 解析：完整 EXE、PATH 上 `.exe`/`.com`/`.cmd`/`.bat`，`.cmd`/`.bat` 走 `%ComSpec% /d /s /c`
- [x] Environment Block：Clone Host → Profile overrides → `ENVBOX_INSTANCE_ID`/`ENVBOX_PROFILE_ID`，Unicode CreateProcess
- [x] Working Directory 与 Arguments 引号语义正确
- [x] Job Object 跟踪 Root 与子进程；可统计/停止（Stop 可后置到 GUI，但 Job API 就绪）
- [x] Startup Fail Policy：Profile 缺失/损坏、无法创建进程时明确失败
- [x] CLI 可 demo：`envbox run --profile us .\envbox-probe.exe` 在**未注入**时至少证明 env/生命周期（或明确尚未虚拟化 API）

## Comments

- Ticket 03 delivered. `envbox run --profile <uuid|name>` launches with case-insensitive Profile env merge + ENVBOX_* IDs, Job Object (KILL_ON_CLOSE, terminate/stats), Fail Policy. Cmd/bat via ComSpec with correct quoting. Command resolve is file-only (rejects dirs/missing .cmd). PATH search uses merged Profile env.
- Review fixes after two-axis review: CreateProcessW + real Unicode lpEnvironment; CREATE_SUSPENDED → assign job → Resume (no early-child race); SafeHandle RAII for process/thread/job; GetLastError captured immediately; full validate_profile on load; AGENTS.md notes name lookup. `cargo test --workspace` 42 passed.

