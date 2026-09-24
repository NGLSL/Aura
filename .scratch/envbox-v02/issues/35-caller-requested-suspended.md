Parent: .scratch/envbox-v02/spec.md

# 35: 边界 — caller_requested_suspended 自动化

**What to build:** 自动化证明注入后不 Resume。

**Blocked by:** （无）

**Status:** resolved

- [x] helper：`CREATE_SUSPENDED` 创建子进程
- [x] 注入后仍 suspended（或调用方 Resume 后才跑）
- [x] Probe/CLI 集成测试

## Comments

### 2026-09-25 implementation (boundary 30–35)

- 新工具 `tools/envbox-suspended-helper`（独立 bin，不改 probe 主逻辑）。
- 行为：`CreateProcessW(CREATE_SUSPENDED)` 创建子进程 → `NtQueryInformationThread(ThreadSuspendCount)` 读挂起计数 → 默认 `ResumeThread` 并 wait；`--hold` 则不 Resume 并 `TerminateProcess`。
- V0.1 语义保持：`hooks_process.cpp` 的 `caller_requested_suspended` 分支不自动 Resume（未改该逻辑）。
- **Evidence**：`cli_boundary.rs`
  - `t35_caller_requested_suspended_stays_suspended_until_resume`：`SUSPEND_COUNT=1`、`STILL_SUSPENDED`、`RESUME_THREAD_PREV=1`、Resume 后 `EnvBox Runtime Loaded` + Profile 继承
  - `t35_without_resume_child_does_not_run`：`--hold` 时 `HELD_NO_RESUME` 且无 probe body（子进程未跑）
- 构建：`cargo build -p envbox-suspended-helper`
