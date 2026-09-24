Parent: .scratch/envbox-v01/spec.md

# 07: Locale / Language / 时区换算补全

**What to build:** Profile 的 Locale、UI Language 列表、GeoID、时区转换在更多 API 上保持逻辑一致，禁止同一进程出现 API 间矛盾（如一处 en-US 一处 zh-CN）。

**Blocked by:** 05 Minimal Profile — 4 个核心 API

**Status:** ready-for-agent

- [ ] 补齐 P0/P1：`GetSystemDefaultLocaleName`、LCID 映射、`GetLocaleInfoEx/W`、`GetUserGeoID`、其余 Preferred UI Languages API、`GetTimeZoneInformation` / `GetTimeZoneInformationForYear`、`SystemTimeToTzSpecificLocalTime(Ex)` / 反向转换
- [ ] Preferred UI Languages 首项为 Profile 语言
- [ ] 时区换算使用 Windows 规则，不手写 DST；真实时间线不变
- [ ] Probe 扩展字段全部与 Profile 一致；Fail Open
