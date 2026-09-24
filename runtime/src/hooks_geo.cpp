// Geo hooks (ticket 05+07). GetUserDefaultGeoName + GetUserGeoID consistency.
// Size queries return Profile length (never Host). Unknown region GeoID returns
// GEOID_NOT_FOUND so it cannot disagree with a Profile GeoName.

#include "hooks.h"

#include <wchar.h>

#ifndef GEOCLASS_NATION
#define GEOCLASS_NATION 16
#endif

static int(WINAPI* TrueGetUserDefaultGeoName)(LPWSTR, int) = GetUserDefaultGeoName;
static GEOID(WINAPI* TrueGetUserGeoID)(GEOCLASS) = GetUserGeoID;

// GEOCLASS_NATION (16) GEOID values for Profile regions (independent literals).
static GEOID GeoIdFromRegion(const wchar_t* region) {
  if (region == nullptr || region[0] == L'\0') {
    return 0;
  }
  struct Entry {
    const wchar_t* iso;
    GEOID id;
  };
  static const Entry kTable[] = {
      {L"US", 244}, {L"CN", 45},  {L"GB", 242}, {L"JP", 122},
      {L"DE", 94},  {L"FR", 77},  {L"CA", 39},  {L"AU", 12},
      {L"IN", 113}, {L"SG", 215}, {L"HK", 104}, {L"TW", 227},
  };
  for (const Entry& e : kTable) {
    if (_wcsicmp(region, e.iso) == 0) {
      return e.id;
    }
  }
  return 0;
}

static int CopyProfileGeoName(const wchar_t* src, LPWSTR dst, int cch) {
  size_t need = wcslen(src) + 1;
  if (dst == nullptr || cch <= 0) {
    return (int)need;
  }
  if ((int)need > cch) {
    SetLastError(ERROR_INSUFFICIENT_BUFFER);
    return 0;
  }
  wcscpy_s(dst, (size_t)cch, src);
  return (int)need;
}

static int WINAPI HookGetUserDefaultGeoName(LPWSTR lpGeoName, int cchGeoName) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr && pfl->has_region) {
    return CopyProfileGeoName(pfl->region, lpGeoName, cchGeoName);
  }
  return TrueGetUserDefaultGeoName(lpGeoName, cchGeoName);
}

static GEOID WINAPI HookGetUserGeoID(GEOCLASS GeoClass) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr && pfl->has_region && GeoClass == GEOCLASS_NATION) {
    // Unknown region -> GEOID_NOT_FOUND (0), never Host (would contradict GeoName).
    return GeoIdFromRegion(pfl->region);
  }
  return TrueGetUserGeoID(GeoClass);
}

int EnvBoxInstallGeoHooks() {
  int ok = 0;
  ok += EnvBoxAttach(&TrueGetUserDefaultGeoName, HookGetUserDefaultGeoName);
  ok += EnvBoxAttach(&TrueGetUserGeoID, HookGetUserGeoID);
  return ok;
}