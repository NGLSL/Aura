// Geo hooks (ticket 05). Primary: GetUserDefaultGeoName (ISO 3166-1 alpha-2).

#include "hooks.h"

#include <wchar.h>

static int(WINAPI* TrueGetUserDefaultGeoName)(LPWSTR, int) = GetUserDefaultGeoName;

static int WINAPI HookGetUserDefaultGeoName(LPWSTR lpGeoName, int cchGeoName) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (lpGeoName != nullptr && cchGeoName > 0 && pfl != nullptr && pfl->has_region) {
    size_t len = wcslen(pfl->region);
    if ((int)len + 1 > cchGeoName) {
      // Fail Open on buffer too small.
      return TrueGetUserDefaultGeoName(lpGeoName, cchGeoName);
    }
    wcscpy_s(lpGeoName, (size_t)cchGeoName, pfl->region);
    return (int)(len + 1);
  }
  return TrueGetUserDefaultGeoName(lpGeoName, cchGeoName);
}

int EnvBoxInstallGeoHooks() {
  return EnvBoxAttach(&TrueGetUserDefaultGeoName, HookGetUserDefaultGeoName);
}
