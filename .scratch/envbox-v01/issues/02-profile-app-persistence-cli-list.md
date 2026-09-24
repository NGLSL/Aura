Parent: .scratch/envbox-v01/spec.md

# 02: Profile & Application 持久化 + 校验 + CLI List

**What to build:** 用户能定义 Environment Profile 与 Application 并保存到 `%LOCALAPPDATA%\EnvBox\`，非法配置不能落盘；通过 `envbox profile list` / `app list` 查看，无需 GUI。

**Blocked by:** 01 Scaffold Workspace + Host Probe 基线

**Status:** ready-for-agent

- [ ] Application / Environment Profile 领域模型与 TOML 往返一致
- [ ] Profile 校验：Locale、Region、Timezone Windows ID、DNS 地址、环境变量名不合法则拒绝保存
- [ ] `envbox profile list` / `envbox app list` 输出可读且含稳定 id
- [ ] 领域术语与 `docs/CONTEXT.md` 一致
- [ ] `cargo test` 覆盖序列化、校验、失败路径
