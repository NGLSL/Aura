Parent: .scratch/envbox-v01/spec.md

# 01: Scaffold Workspace + Host Probe 基线

**What to build:** 开发者能在 Windows 上一次构建出 EnvBox 骨架，并用 `envbox-probe` 得到 Host 的 GEO / LOCALE / LANGUAGE / TIMEZONE / DNS / ENV 快照，作为后续 Profile 对比的独立基线。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] Rust workspace（core / storage / launcher / cli）可 `cargo build`
- [x] Runtime CMake/MSVC + Microsoft Detours 工程可编译出 `envbox-runtime32/64.dll`（可先空实现）
- [x] `envbox-probe` 可执行并打印 GEO/LOCALE/LANGUAGE/TIMEZONE/DNS/ENV
- [x] Probe `--spawn-child` 能再启动一份自身并输出（先证明父子进程机制，不要求 Profile）
- [x] 不修改任何 Host 系统配置

## Comments

- 2026: Ticket 01 delivered. `cargo test --workspace` 12 passed; probe Host snapshot verified on this machine (CN/zh-CN/China Standard Time). Runtime 32/64 empty DLLs built via CMake+MSVC. Detours link flag ready (`ENVBOX_WITH_DETOURS`) and lands with ticket 04. Code review: Standards/Spec axes reviewed; fixed ENV full dump, timezone error path, CONTEXT `whitelist_paths`, dropped unused tracing.
