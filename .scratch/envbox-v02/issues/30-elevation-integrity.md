Parent: .scratch/envbox-v02/spec.md

# 30: 边界 — 提权/完整性级别

**What to build:** 高完整性子进程注入失败时 Startup Fail Policy，明确错误。

**Blocked by:** （可与 Audit 并行，建议后）

**Status:** resolved

- [x] 无法注入的提权子进程不静默无虚拟化运行
- [x] 错误信息说明所需完整性级别
- [x] README 补充说明

## Comments

### 2026-09-25 implementation (boundary 30–35)

- **Startup Fail Policy**：`launch()` 注入失败直接 `Err`，不回落 plain CreateProcess（既有契约保持）。
- **错误映射**（`crates/envbox-launcher/src/injection.rs`）：
  - `ERROR_ELEVATION_REQUIRED(740)` / `ERROR_ACCESS_DENIED(5)` / `ERROR_PRIVILEGE_NOT_HELD(1314)` → `InjectError::ElevationIntegrity`
  - 文案含 `integrity/elevation` 与 `same integrity level as EnvBox or lower`
  - `map_create_process_error` 已接到 `DetourCreateProcessWithDllExW` 失败路径（`launcher.rs`）
- **README**：`Security posture` 下新增 “Elevation / integrity” 小节（非安全边界声明保留）。
- **Evidence**
  - 单测：`envbox-launcher` → `elevation_errors_map_with_integrity_guidance`、`bad_exe_format_maps_to_architecture_load`
  - 集成：`cli_boundary.rs` → `t30_inject_failure_never_silently_runs_unvirtualized`、`t30_elevated_target_fails_with_integrity_guidance_when_available`（无真 requireAdministrator 工具时 skip）
  - 命令：`cargo test -p envbox-launcher`（32 ok）；`cargo test -p envbox-cli --test cli_boundary`（14 ok）
