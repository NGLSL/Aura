Parent: .scratch/envbox-v02/spec.md

# 32: 边界 — 多级 cmd/bat wrapper

**What to build:** wrapper → node → child 全链路 Profile 继承。

**Blocked by:** （无）

**Status:** resolved

- [x] 多级 `.cmd`/`.bat` 全部带 Profile
- [x] PATH 解析与 `.cmd` 优先规则不回退
- [x] 矩阵补充多级 wrapper 用例

## Comments

### 2026-09-25 implementation (boundary 30–35)

- 测试夹具：`outer.cmd` → `inner.cmd` → `envbox-probe --child`（临时目录）。
- 断言叶子进程：`EnvBox Runtime Loaded`、`ENVBOX_PROFILE_ID`、`ENVBOX_BOUNDARY_MARK`、Geo/Locale/Timezone 与 Profile 一致。
- PATH 规则：目录内同时放非 PE shim（`edge-tool`）与 `edge-tool.cmd`；经 PATH 解析必须走 `.cmd`（ComSpec 包装），叶子仍带 Profile。
- 复用现有 `command.rs` 语义（`.cmd`/`.bat` → `%ComSpec% /d /s /c`；扩展名优先于 extension-less shim），无新回退。
- **Evidence**：`cli_boundary.rs` → `t32_multilevel_cmd_wrappers_keep_profile`、`t32_path_resolution_prefers_cmd_wrapper`（裸命令 `edge-tool` 走 `resolve_command` PATH 选择 `.cmd`，不再显式 `call edge-tool.cmd`）
