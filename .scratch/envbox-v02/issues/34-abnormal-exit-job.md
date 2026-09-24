Parent: .scratch/envbox-v02/spec.md

# 34: 边界 — 异常退出 / Job Object

**What to build:** 崩溃、Terminate、Job close 无句柄泄漏；终态正确。

**Blocked by:** （无）

**Status:** resolved

- [x] 异常退出后 InstanceStatus ∈ Exited/Failed
- [x] 进程/线程/Job 句柄 RAII，无泄漏
- [x] `KILL_ON_JOB_CLOSE` Stop 语义保持
- [x] 自动化覆盖

## Comments

### 2026-09-25 implementation (boundary 30–35)

- **语义复核（未改代码）**
  - `job.rs`：`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 创建时设置；`close()` Drop SafeHandle（Stop 契约，不用 TerminateJobObject）
  - `launcher.rs`：`SpawnedChild` / `LaunchedProcess` 的 process/thread 为 `SafeHandle`（RAII `CloseHandle`）；`GetLastError` 紧随失败调用保存
  - `instance.rs`：`status_from_stats` 空 Job → `Exited`；Stop 失败 → `Failed`（不卡 Stopping）
- **Evidence**：`cli_boundary.rs`
  - `t34_abnormal_exit_status_is_terminal`（`cmd /c exit 7`）
  - `t34_self_terminate_status_is_terminal`（PowerShell `Process.Kill()` = TerminateProcess 自杀，非 `exit N`）
  - `t34_wait_returns_abnormal_exit_code`（`wait()` 非 success）
  - `t34_stop_kills_tree_without_residual_process`（Stop 后 `tasklist` 无残留 pid）
