Parent: .scratch/envbox-v01/spec.md

# 01: Scaffold Workspace + Host Probe 基线

**What to build:** 开发者能在 Windows 上一次构建出 EnvBox 骨架，并用 `envbox-probe` 得到 Host 的 GEO / LOCALE / LANGUAGE / TIMEZONE / DNS / ENV 快照，作为后续 Profile 对比的独立基线。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Rust workspace（core / storage / launcher / cli）可 `cargo build`
- [ ] Runtime CMake/MSVC + Microsoft Detours 工程可编译出 `envbox-runtime32/64.dll`（可先空实现）
- [ ] `envbox-probe` 可执行并打印 GEO/LOCALE/LANGUAGE/TIMEZONE/DNS/ENV
- [ ] Probe `--spawn-child` 能再启动一份自身并输出（先证明父子进程机制，不要求 Profile）
- [ ] 不修改任何 Host 系统配置
