// Timezone hooks (ticket 05). Virtual timezone, real timeline:
// never hook GetSystemTime / QPC / GetTickCount; never SetDynamicTimeZoneInformation.

#include "hooks.h"

#include <wchar.h>

static DWORD(WINAPI* TrueGetDynamicTimeZoneInformation)(
    PDYNAMIC_TIME_ZONE_INFORMATION) = GetDynamicTimeZoneInformation;

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

// Classify STANDARD vs DAYLIGHT for the Profile zone using real UTC and the
// zone's own transition rules (never SetDynamicTimeZoneInformation).
static DWORD TimeZoneIdFromInfo(const DYNAMIC_TIME_ZONE_INFORMATION* info) {
  if (info->DaylightDate.wMonth == 0) {
    return TIME_ZONE_ID_STANDARD;
  }

  TIME_ZONE_INFORMATION with_dst = {};
  TIME_ZONE_INFORMATION std_only = {};
  DynamicToClassic(info, &with_dst);
  DynamicToClassic(info, &std_only);
  std_only.DaylightDate = {};  // suppress DST window
  std_only.DaylightBias = std_only.StandardBias;

  SYSTEMTIME utc = {};
  GetSystemTime(&utc);  // real timeline

  SYSTEMTIME local_dst = {};
  SYSTEMTIME local_std = {};
  if (!SystemTimeToTzSpecificLocalTime(&with_dst, &utc, &local_dst) ||
      !SystemTimeToTzSpecificLocalTime(&std_only, &utc, &local_std)) {
    // Fail Open-ish: cannot classify; report STANDARD rather than INVALID.
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

static DWORD WINAPI HookGetDynamicTimeZoneInformation(
    PDYNAMIC_TIME_ZONE_INFORMATION p) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (p != nullptr && pfl != nullptr && pfl->has_tz) {
    DYNAMIC_TIME_ZONE_INFORMATION info = {};
    if (EnvBoxLookupTimeZone(pfl->tz_windows, &info)) {
      *p = info;
      return TimeZoneIdFromInfo(&info);
    }
    // Fail Open: lookup miss -> original API.
  }
  return TrueGetDynamicTimeZoneInformation(p);
}

int EnvBoxInstallTimeHooks() {
  return EnvBoxAttach(&TrueGetDynamicTimeZoneInformation,
                      HookGetDynamicTimeZoneInformation);
}
