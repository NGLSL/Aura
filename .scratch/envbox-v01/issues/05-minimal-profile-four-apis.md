Parent: .scratch/envbox-v01/spec.md

# 05: Minimal Profile — 4 个核心 API

**What to build:** 用户用 US Profile 启动 Probe 时，时区/Geo/Locale/UI Language 四类 API 返回 Profile 值；与直接跑 Probe 的 Host 快照对比差异清晰，且 Host 配置不变。单点 Hook 失败 Fail Open，不崩目标进程。

**Blocked by:** 04 Detours 注入烟测

**Status:** done

- [x] Hook：`GetDynamicTimeZoneInformation`、`GetUserDefaultGeoName`、`GetUserDefaultLocaleName`、`GetUserDefaultUILanguage`
- [x] Probe Host 快照 vs `envbox run --profile us` 快照：上述字段变为 Profile 值（如 Pacific / US / en-US）
- [x] 非虚拟化字段与 Host 基线一致（独立期望值，非重算）
- [x] Profile 运行期 immutable（init 一次）
- [x] Hook 错误回退原 API；不修改 `GetSystemTime`/`QPC`/`GetTickCount` 等真实时间
- [x] 不调用 `SetDynamicTimeZoneInformation`

## Comments

- Ticket 05 delivered. Four Detours hooks in split modules (`hooks_time/geo/locale/language.cpp`). `RuntimeProfile` loads once from `profiles.toml` (line-anchored key extract) into an immutable static; `has_*` fields gate Fail Open fallbacks.
- Probe Host vs `envbox run --profile us`: `GetUserDefaultGeoName=US`, `GetUserDefaultLocaleName=en-US`, `GetUserDefaultUILanguage=0x0409`, `GetDynamicTimeZoneInformation=Pacific Standard Time`. Non-virtualized fields (`GetSystemDefaultLocaleName`, `GetSystemDefaultUILanguage`, `GetUserGeoID`, `GetUserDefaultLCID`, preferred UI list, DNS) match Host baseline.
- Review fixes: missing `profiles.toml` with `ENVBOX_CONFIG_ROOT` is fatal (Startup Fail Policy); `GetLastError` saved on open/read; timezone hook returns STANDARD/DAYLIGHT (not always UNKNOWN); no `DetourTransactionAbort` after failed Commit; tests use exact fixture literals + Host contrast `assert_ne`.
- Still Host (by design, later tickets): `GetUserGeoID`, LCIDs, `GetSystemDefault*`, preferred-UI-language list order, formatting APIs, DNS, registry, child propagation.
- Evidence: `cargo test --workspace` 51 passed including `run_probe_four_core_apis_show_profile_values` (exact + contrast) and `run_probe_non_virtualized_fields_match_host`.

