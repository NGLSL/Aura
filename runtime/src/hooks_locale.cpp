// Locale hooks (ticket 05). Primary: GetUserDefaultLocaleName.

#include "hooks.h"

#include <wchar.h>

static int(WINAPI* TrueGetUserDefaultLocaleName)(LPWSTR, int) =
    GetUserDefaultLocaleName;

static int WINAPI HookGetUserDefaultLocaleName(LPWSTR lpLocaleName,
                                               int cchLocaleName) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (lpLocaleName != nullptr && cchLocaleName > 0 && pfl != nullptr &&
      pfl->has_locale) {
    size_t len = wcslen(pfl->locale_name);
    if ((int)len + 1 > cchLocaleName) {
      return TrueGetUserDefaultLocaleName(lpLocaleName, cchLocaleName);
    }
    wcscpy_s(lpLocaleName, (size_t)cchLocaleName, pfl->locale_name);
    return (int)(len + 1);
  }
  return TrueGetUserDefaultLocaleName(lpLocaleName, cchLocaleName);
}

int EnvBoxInstallLocaleHooks() {
  return EnvBoxAttach(&TrueGetUserDefaultLocaleName, HookGetUserDefaultLocaleName);
}
