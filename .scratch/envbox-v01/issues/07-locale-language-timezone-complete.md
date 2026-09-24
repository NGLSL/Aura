Parent: .scratch/envbox-v01/spec.md

# 07: Locale / Language / 时区换算补全

**What to build:** Profile 的 Locale、UI Language 列表、GeoID、时区转换在更多 API 上保持逻辑一致，禁止同一进程出现 API 间矛盾（如一处 en-US 一处 zh-CN）。

**Blocked by:** 05 Minimal Profile — 4 个核心 API

**Status:** done

- [x] 补齐 P0/P1：`GetSystemDefaultLocaleName`、LCID 映射、`GetLocaleInfoEx/W`、`GetUserGeoID`、其余 Preferred UI Languages API、`GetTimeZoneInformation` / `GetTimeZoneInformationForYear`、`SystemTimeToTzSpecificLocalTime(Ex)` / 反向转换
- [x] Preferred UI Languages 首项为 Profile 语言
- [x] 时区换算使用 Windows 规则，不手写 DST；真实时间线不变
- [x] Probe 扩展字段全部与 Profile 一致；Fail Open

## Comments

- Ticket 07 delivered. Locale: User/System default locale name + LCID + GetLocaleInfoEx/W (NULL=`LOCALE_NAME_USER_DEFAULT`, `L"!"`=system; `L""`/`LOCALE_NEUTRAL` left alone). Language: User/System/Thread/Process preferred UI lists with Profile token first (name form or `MUI_LANGUAGE_ID` hex). Geo: `GetUserGeoID(GEOCLASS_NATION)` table; unknown region -> GEOID_NOT_FOUND (never Host, which would contradict GeoName). Time: GetTimeZoneInformation / GetTimeZoneInformationForYear + SystemTimeToTzSpecificLocalTime(Ex) reverse; NULL zone means current -> Profile; Windows rules only (no hand-rolled DST); real timeline unchanged.
- Review fixes (hard/wrong): size queries return Profile length (never Host name on Fail Open); Preferred-UI size-query count is post-dedupe; MUI_LANGUAGE_ID uses hex LANGIDs; INVARIANT/NEUTRAL not remapped; dllmain log says `attached=`.
- Evidence: `cargo test --workspace` 57 passed — `run_probe_locale_language_timezone_consistent_with_profile` covers GeoID 244, en-US LCID 1033, SNAME Ex/W, UI 0x0409, preferred lists (User/System/Thread/Process + ID form 0409), TZ key + classic/year StandardName agreement, Bias match, conversion fixture 2024-01-15 12:00 UTC <-> 04:00 local (non-Ex + Ex).
- Deferred: GeoID table is 12 regions (US fixture proven); other regions use GEOID_NOT_FOUND. Locale-name size-query contract is unit-covered by behavior in integration, not a dedicated buffer-too-small harness.