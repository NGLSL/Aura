Parent: .scratch/envbox-v01/spec.md

# 05: Minimal Profile — 4 个核心 API

**What to build:** 用户用 US Profile 启动 Probe 时，时区/Geo/Locale/UI Language 四类 API 返回 Profile 值；与直接跑 Probe 的 Host 快照对比差异清晰，且 Host 配置不变。单点 Hook 失败 Fail Open，不崩目标进程。

**Blocked by:** 04 Detours 注入烟测

**Status:** ready-for-agent

- [ ] Hook：`GetDynamicTimeZoneInformation`、`GetUserDefaultGeoName`、`GetUserDefaultLocaleName`、`GetUserDefaultUILanguage`
- [ ] Probe Host 快照 vs `envbox run --profile us` 快照：上述字段变为 Profile 值（如 Pacific / US / en-US）
- [ ] 非虚拟化字段与 Host 基线一致（独立期望值，非重算）
- [ ] Profile 运行期 immutable（init 一次）
- [ ] Hook 错误回退原 API；不修改 `GetSystemTime`/`QPC`/`GetTickCount` 等真实时间
- [ ] 不调用 `SetDynamicTimeZoneInformation`
