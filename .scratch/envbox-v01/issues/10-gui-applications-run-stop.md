Parent: .scratch/envbox-v01/spec.md

# 10: GUI — Application 管理 + Run/Stop

**What to build:** 用户在 GUI 中 Add/Edit/Delete Application，一键 Run 产生 RuntimeInstance，Stop 结束整棵 Process Tree Instance，并能看到状态与子进程数量。GUI 不包含注入逻辑。

**Blocked by:** 06 子进程传播（Process Tree Instance）

**Status:** done

- [x] Application 字段：Name、Launch type（Executable/Command）、路径/命令、Arguments、Working Directory、Default Profile、子进程继承开关
- [x] Save / Run / Delete 可用；配置写入 storage
- [x] Run 创建 RuntimeInstance；状态 Starting/Running/Stopping/Exited/Failed
- [x] Root 退出但 Job 内仍有子进程时保持 Running
- [x] Stop 关闭 Job（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 语义）并更新状态
- [x] GUI 仅调用 core/launcher

## Comments

### 2026-09-24 implementation + dual-axis review (general-11 → fix → general-12)

**Evidence**
- GUI: `crates/envbox-app/src/main.rs` — New/Save/Delete/Run；Arguments 用 `format_args`/`parse_args`（CommandLineToArgvW，非 join/split_whitespace）；保存前校验 `default_profile_id` 必须存在。
- InstanceManager: `crates/envbox-launcher/src/instance.rs` — `RunTarget::{Profile,Host}`；`status_from_stats`：有子进程→Running，空 Job→Exited（fast-exit 非 Failed）；Root 退出仍有子进程保持 Running。
- Stop: `LaunchedProcess::stop` → `InstanceJob::close()`（`KILL_ON_JOB_CLOSE`），失败→`InstanceError::Job` 且状态 Failed（不冻结 Stopping）。
- 时区枚举：GUI 调 `envbox_storage::enumerate_dynamic_timezone_ids`，无本地 Win32 重复实现。
- 测试：`status_from_stats_*`、`host_target_builds_unvirtualized_launch_request`、`run_target_host_profile_id_is_nil_*`、`parse_format_args_round_trip`。`cargo test --workspace` **74 passed**。

**Review**
- general-11 Hard=3 / Wrong=5 → 已修。
- general-12 Hard=0；Wrong=2（stale IANA、fast-exit Failed）→ 已修；Fowler/Missing 残留见下。

**Accepted V0.1 residuals（非 hard/wrong）**
- Starting/Stopping 为状态机中间态；GUI 同步 run/stop，列表靠手动 Refresh 刷新（ticket 允许看到状态与子进程数；无自动轮询）。
- `AppRunWith(Uuid)` 仍用 `nil` 表示 Host 菜单项（内部已映射 `RunTarget::Host`，不伪造 Profile）。
