// Timezone hooks (ticket 05+07). Virtual timezone, real timeline:
// never hook GetSystemTime / QPC / GetTickCount; never SetDynamicTimeZoneInformation.
// Conversions use Windows rules (SystemTimeToTzSpecificLocalTime* family).

#include "hooks.h"

#include <wchar.h>

#include "audit.h"

static DWORD(WINAPI* TrueGetDynamicTimeZoneInformation)(
    PDYNAMIC_TIME_ZONE_INFORMATION) = GetDynamicTimeZoneInformation;
static DWORD(WINAPI* TrueGetTimeZoneInformation)(LPTIME_ZONE_INFORMATION) =
    GetTimeZoneInformation;
static BOOL(WINAPI* TrueGetTimeZoneInformationForYear)(
    USHORT, PDYNAMIC_TIME_ZONE_INFORMATION, LPTIME_ZONE_INFORMATION) =
    GetTimeZoneInformationForYear;
static BOOL(WINAPI* TrueSystemTimeToTzSpecificLocalTime)(
    const TIME_ZONE_INFORMATION*, const SYSTEMTIME*, LPSYSTEMTIME) =
    SystemTimeToTzSpecificLocalTime;
static BOOL(WINAPI* TrueSystemTimeToTzSpecificLocalTimeEx)(
    const DYNAMIC_TIME_ZONE_INFORMATION*, const SYSTEMTIME*, LPSYSTEMTIME) =
    SystemTimeToTzSpecificLocalTimeEx;
static BOOL(WINAPI* TrueTzSpecificLocalTimeToSystemTime)(
    const TIME_ZONE_INFORMATION*, const SYSTEMTIME*, LPSYSTEMTIME) =
    TzSpecificLocalTimeToSystemTime;
static BOOL(WINAPI* TrueTzSpecificLocalTimeToSystemTimeEx)(
    const DYNAMIC_TIME_ZONE_INFORMATION*, const SYSTEMTIME*, LPSYSTEMTIME) =
    TzSpecificLocalTimeToSystemTimeEx;

static void DynamicToClassic(const DYNAMIC_TIME_ZONE_INFORMATION* din,
                             TIME_ZONE_INFORMATION* tout) {
  tout->Bias = din->Bias;
  tout->StandardBias = din->StandardBias;
  tout->DaylightBias = din->DaylightBias;
  tout->StandardDate = din->StandardDate;
  tout->DaylightDate = din->DaylightDate;
  wcsncpy_s(tout->StandardName, din->StandardName, _TRUNCATE);
  wcsncpy_s(tout->DaylightName, din->DaylightName, _TRUNCATE);
}

static int FillProfileDynamic(DYNAMIC_TIME_ZONE_INFORMATION* out) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_tz || out == nullptr) {
    return 0;
  }
  return EnvBoxLookupTimeZone(pfl->tz_windows, out);
}

static DWORD TimeZoneIdFromInfo(const DYNAMIC_TIME_ZONE_INFORMATION* info) {
  if (info->DaylightDate.wMonth == 0) {
    return TIME_ZONE_ID_STANDARD;
  }
  TIME_ZONE_INFORMATION with_dst = {};
  TIME_ZONE_INFORMATION std_only = {};
  DynamicToClassic(info, &with_dst);
  DynamicToClassic(info, &std_only);
  std_only.DaylightDate = {};
  std_only.DaylightBias = std_only.StandardBias;

  SYSTEMTIME utc = {};
  GetSystemTime(&utc);  // real timeline

  SYSTEMTIME local_dst = {};
  SYSTEMTIME local_std = {};
  if (!SystemTimeToTzSpecificLocalTime(&with_dst, &utc, &local_dst) ||
      !SystemTimeToTzSpecificLocalTime(&std_only, &utc, &local_std)) {
    return TIME_ZONE_ID_STANDARD;
  }
  FILETIME f_dst = {};
  FILETIME f_std = {};
  if (!SystemTimeToFileTime(&local_dst, &f_dst) ||
      !SystemTimeToFileTime(&local_std, &f_std)) {
    return TIME_ZONE_ID_STANDARD;
  }
  if (f_dst.dwLowDateTime != f_std.dwLowDateTime ||
      f_dst.dwHighDateTime != f_std.dwHighDateTime) {
    return TIME_ZONE_ID_DAYLIGHT;
  }
  return TIME_ZONE_ID_STANDARD;
}

// TimeZone ID is derived from rules + current UTC; cache per process because
// Chrome calls GetTimeZoneInformation in tight loops.
static DWORD CachedTimeZoneId(const DYNAMIC_TIME_ZONE_INFORMATION* info) {
  static DYNAMIC_TIME_ZONE_INFORMATION s_last = {};
  static DWORD s_id = TIME_ZONE_ID_STANDARD;
  static int s_ready = 0;
  if (s_ready && memcmp(&s_last, info, sizeof(s_last)) == 0) {
    return s_id;
  }
  s_id = TimeZoneIdFromInfo(info);
  s_last = *info;
  s_ready = 1;
  return s_id;
}

static DWORD WINAPI HookGetDynamicTimeZoneInformation(
    PDYNAMIC_TIME_ZONE_INFORMATION p) {
  DYNAMIC_TIME_ZONE_INFORMATION info = {};
  if (p != nullptr && FillProfileDynamic(&info)) {
    *p = info;
    EnvBoxAuditEventW("GetDynamicTimeZoneInformation", 1, info.TimeZoneKeyName);
    return CachedTimeZoneId(&info);
  }
  EnvBoxAuditEvent("GetDynamicTimeZoneInformation", 0, nullptr);
  return TrueGetDynamicTimeZoneInformation(p);
}

static DWORD WINAPI HookGetTimeZoneInformation(
    LPTIME_ZONE_INFORMATION lpTimeZoneInformation) {
  DYNAMIC_TIME_ZONE_INFORMATION dyn = {};
  if (lpTimeZoneInformation != nullptr && FillProfileDynamic(&dyn)) {
    DynamicToClassic(&dyn, lpTimeZoneInformation);
    EnvBoxAuditEventW("GetTimeZoneInformation", 1, dyn.TimeZoneKeyName);
    return CachedTimeZoneId(&dyn);
  }
  EnvBoxAuditEvent("GetTimeZoneInformation", 0, nullptr);
  return TrueGetTimeZoneInformation(lpTimeZoneInformation);
}

static BOOL WINAPI HookGetTimeZoneInformationForYear(
    USHORT wYear, PDYNAMIC_TIME_ZONE_INFORMATION pdtzi,
    LPTIME_ZONE_INFORMATION ptzi) {
  if (pdtzi == nullptr) {
    DYNAMIC_TIME_ZONE_INFORMATION dyn = {};
    if (FillProfileDynamic(&dyn)) {
      EnvBoxAuditEventW("GetTimeZoneInformationForYear", 1, dyn.TimeZoneKeyName);
      return TrueGetTimeZoneInformationForYear(wYear, &dyn, ptzi);
    }
  }
  EnvBoxAuditEvent("GetTimeZoneInformationForYear", 0, nullptr);
  return TrueGetTimeZoneInformationForYear(wYear, pdtzi, ptzi);
}

static const TIME_ZONE_INFORMATION* ProfileClassicOrNull(
    const TIME_ZONE_INFORMATION* zone, TIME_ZONE_INFORMATION* scratch) {
  if (zone != nullptr) {
    return zone;
  }
  DYNAMIC_TIME_ZONE_INFORMATION dyn = {};
  if (!FillProfileDynamic(&dyn)) {
    return nullptr;
  }
  DynamicToClassic(&dyn, scratch);
  return scratch;
}

static const DYNAMIC_TIME_ZONE_INFORMATION* ProfileDynamicOrNull(
    const DYNAMIC_TIME_ZONE_INFORMATION* zone,
    DYNAMIC_TIME_ZONE_INFORMATION* scratch) {
  if (zone != nullptr) {
    return zone;
  }
  if (!FillProfileDynamic(scratch)) {
    return nullptr;
  }
  return scratch;
}

// Null lpTimeZoneInformation means "use default zone": that path is virtualized
// to Profile when has_tz. Explicit zone stays caller-owned (not virtualized).
static void AuditTzConvert(const char* api, int null_zone, int used_profile,
                           const wchar_t* tz_key) {
  if (null_zone && used_profile) {
    EnvBoxAuditEventW(api, 1, tz_key ? tz_key : L"tz-default");
  } else {
    EnvBoxAuditEvent(api, 0, nullptr);
  }
}

static BOOL WINAPI HookSystemTimeToTzSpecificLocalTime(
    const TIME_ZONE_INFORMATION* lpTimeZoneInformation,
    const SYSTEMTIME* lpUniversalTime, LPSYSTEMTIME lpLocalTime) {
  TIME_ZONE_INFORMATION scratch = {};
  const TIME_ZONE_INFORMATION* z =
      ProfileClassicOrNull(lpTimeZoneInformation, &scratch);
  const RuntimeProfile* pfl = EnvBoxProfile();
  AuditTzConvert("SystemTimeToTzSpecificLocalTime",
                 lpTimeZoneInformation == nullptr, z != nullptr,
                 (pfl != nullptr && pfl->has_tz) ? pfl->tz_windows : nullptr);
  return TrueSystemTimeToTzSpecificLocalTime(z, lpUniversalTime, lpLocalTime);
}

static BOOL WINAPI HookSystemTimeToTzSpecificLocalTimeEx(
    const DYNAMIC_TIME_ZONE_INFORMATION* lpTimeZoneInformation,
    const SYSTEMTIME* lpUniversalTime, LPSYSTEMTIME lpLocalTime) {
  DYNAMIC_TIME_ZONE_INFORMATION scratch = {};
  const DYNAMIC_TIME_ZONE_INFORMATION* z =
      ProfileDynamicOrNull(lpTimeZoneInformation, &scratch);
  const RuntimeProfile* pfl = EnvBoxProfile();
  AuditTzConvert("SystemTimeToTzSpecificLocalTimeEx",
                 lpTimeZoneInformation == nullptr, z != nullptr,
                 (pfl != nullptr && pfl->has_tz) ? pfl->tz_windows : nullptr);
  return TrueSystemTimeToTzSpecificLocalTimeEx(z, lpUniversalTime, lpLocalTime);
}

static BOOL WINAPI HookTzSpecificLocalTimeToSystemTime(
    const TIME_ZONE_INFORMATION* lpTimeZoneInformation,
    const SYSTEMTIME* lpLocalTime, LPSYSTEMTIME lpUniversalTime) {
  TIME_ZONE_INFORMATION scratch = {};
  const TIME_ZONE_INFORMATION* z =
      ProfileClassicOrNull(lpTimeZoneInformation, &scratch);
  const RuntimeProfile* pfl = EnvBoxProfile();
  AuditTzConvert("TzSpecificLocalTimeToSystemTime",
                 lpTimeZoneInformation == nullptr, z != nullptr,
                 (pfl != nullptr && pfl->has_tz) ? pfl->tz_windows : nullptr);
  return TrueTzSpecificLocalTimeToSystemTime(z, lpLocalTime, lpUniversalTime);
}

static BOOL WINAPI HookTzSpecificLocalTimeToSystemTimeEx(
    const DYNAMIC_TIME_ZONE_INFORMATION* lpTimeZoneInformation,
    const SYSTEMTIME* lpLocalTime, LPSYSTEMTIME lpUniversalTime) {
  DYNAMIC_TIME_ZONE_INFORMATION scratch = {};
  const DYNAMIC_TIME_ZONE_INFORMATION* z =
      ProfileDynamicOrNull(lpTimeZoneInformation, &scratch);
  const RuntimeProfile* pfl = EnvBoxProfile();
  AuditTzConvert("TzSpecificLocalTimeToSystemTimeEx",
                 lpTimeZoneInformation == nullptr, z != nullptr,
                 (pfl != nullptr && pfl->has_tz) ? pfl->tz_windows : nullptr);
  return TrueTzSpecificLocalTimeToSystemTimeEx(z, lpLocalTime, lpUniversalTime);
}

int EnvBoxInstallTimeHooks() {
  int ok = 0;
  ok += EnvBoxAttach(&TrueGetDynamicTimeZoneInformation,
                     HookGetDynamicTimeZoneInformation);
  ok += EnvBoxAttach(&TrueGetTimeZoneInformation, HookGetTimeZoneInformation);
  ok += EnvBoxAttach(&TrueGetTimeZoneInformationForYear,
                     HookGetTimeZoneInformationForYear);
  ok += EnvBoxAttach(&TrueSystemTimeToTzSpecificLocalTime,
                     HookSystemTimeToTzSpecificLocalTime);
  ok += EnvBoxAttach(&TrueSystemTimeToTzSpecificLocalTimeEx,
                     HookSystemTimeToTzSpecificLocalTimeEx);
  ok += EnvBoxAttach(&TrueTzSpecificLocalTimeToSystemTime,
                     HookTzSpecificLocalTimeToSystemTime);
  ok += EnvBoxAttach(&TrueTzSpecificLocalTimeToSystemTimeEx,
                     HookTzSpecificLocalTimeToSystemTimeEx);
  return ok;
}
