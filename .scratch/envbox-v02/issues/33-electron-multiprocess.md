Parent: .scratch/envbox-v02/spec.md

# 33: 边界 — Electron 多进程

**What to build:** Electron main/renderer/utility 全树继承 Profile。

**Blocked by:** 32

**Status:** resolved

- [x] 多级 CreateProcess 全部注入
- [x] utility/gpu/renderer 子进程同一 Environment View
- [x] 无子进程逃逸（抽查）

## Comments

### 2026-09-25 implementation (boundary 30–35)

- 无真 Electron 依赖。`main.cmd` 作为 parent，经 CreateProcess 连续拉起 3 个 `envbox-probe --child`（utility / gpu / renderer 角色标记）。
- 每个子进程走 `hooks_process.cpp` 注入路径（Runtime marker ≥ 3）。
- 抽查无逃逸：每个 child block 的 `ENVBOX_PROFILE_ID` 相同且等于当前 Profile；Geo/Locale/Timezone 均为 Profile 值。
- **Evidence**：`cli_boundary.rs` → `t33_multi_child_all_injected_same_profile`（ok）
