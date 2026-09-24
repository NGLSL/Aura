// Locale hooks (ticket 05+07). Keep user/system default locale names and LCIDs
// consistent with Profile. Fail Open on any error. Size queries return Profile
// length (never Host) so exact-alloc callers cannot silently see Host values.

#include "hooks.h"

#include <wchar.h>

static int(WINAPI* TrueGetUserDefaultLocaleName)(LPWSTR, int) =
    GetUserDefaultLocaleName;
static int(WINAPI* TrueGetSystemDefaultLocaleName)(LPWSTR, int) =
    GetSystemDefaultLocaleName;
static LCID(WINAPI* TrueGetUserDefaultLCID)() = GetUserDefaultLCID;
static LCID(WINAPI* TrueGetSystemDefaultLCID)() = GetSystemDefaultLCID;
static int(WINAPI* TrueGetLocaleInfoEx)(LPCWSTR, LCTYPE, LPWSTR, int) =
    GetLocaleInfoEx;
static int(WINAPI* TrueGetLocaleInfoW)(LCID, LCTYPE, LPWSTR, int) = GetLocaleInfoW;

// Write Profile locale name. Size query (null dst) returns Profile length+1.
// Buffer too small returns 0 + ERROR_INSUFFICIENT_BUFFER (never Host name).
static int CopyProfileLocaleName(const wchar_t* src, LPWSTR dst, int cch) {
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

static int WINAPI HookGetUserDefaultLocaleName(LPWSTR lpLocaleName,
                                               int cchLocaleName) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr && pfl->has_locale) {
    return CopyProfileLocaleName(pfl->locale_name, lpLocaleName, cchLocaleName);
  }
  return TrueGetUserDefaultLocaleName(lpLocaleName, cchLocaleName);
}

static int WINAPI HookGetSystemDefaultLocaleName(LPWSTR lpLocaleName,
                                                 int cchLocaleName) {
  // Environment-consistent: same Profile locale as the user default.
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr && pfl->has_locale) {
    return CopyProfileLocaleName(pfl->locale_name, lpLocaleName, cchLocaleName);
  }
  return TrueGetSystemDefaultLocaleName(lpLocaleName, cchLocaleName);
}

static LCID ProfileLocaleLcid() {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_locale) {
    return 0;
  }
  return LocaleNameToLCID(pfl->locale_name, 0);
}

static LCID WINAPI HookGetUserDefaultLCID() {
  LCID lcid = ProfileLocaleLcid();
  return lcid != 0 ? lcid : TrueGetUserDefaultLCID();
}

static LCID WINAPI HookGetSystemDefaultLCID() {
  LCID lcid = ProfileLocaleLcid();
  return lcid != 0 ? lcid : TrueGetSystemDefaultLCID();
}

// LOCALE_USER_DEFAULT (0x0400) / LOCALE_SYSTEM_DEFAULT (0x0800) map to Profile.
// LOCALE_NEUTRAL (0) is a real query - leave it alone (not a "default" alias).
static LCID VirtualizeLcid(LCID lcid) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_locale) {
    return lcid;
  }
  if (lcid == LOCALE_USER_DEFAULT || lcid == LOCALE_SYSTEM_DEFAULT) {
    LCID p = ProfileLocaleLcid();
    return p != 0 ? p : lcid;
  }
  return lcid;
}

static int WINAPI HookGetLocaleInfoW(LCID Locale, LCTYPE LCType, LPWSTR lpLCData,
                                     int cchData) {
  return TrueGetLocaleInfoW(VirtualizeLcid(Locale), LCType, lpLCData, cchData);
}

// LOCALE_NAME_USER_DEFAULT is NULL; LOCALE_NAME_SYSTEM_DEFAULT is L"!".
// LOCALE_NAME_INVARIANT is L"" - a real locale name, not a default alias.
static LPCWSTR VirtualizeLocaleName(LPCWSTR name, const RuntimeProfile* pfl) {
  if (pfl == nullptr || !pfl->has_locale) {
    return name;
  }
  if (name == nullptr || (name[0] == L'!' && name[1] == L'\0')) {
    return pfl->locale_name;
  }
  return name;
}

static int WINAPI HookGetLocaleInfoEx(LPCWSTR lpLocaleName, LCTYPE LCType,
                                      LPWSTR lpLCData, int cchData) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  return TrueGetLocaleInfoEx(VirtualizeLocaleName(lpLocaleName, pfl), LCType,
                             lpLCData, cchData);
}

int EnvBoxInstallLocaleHooks() {
  int ok = 0;
  ok += EnvBoxAttach(&TrueGetUserDefaultLocaleName, HookGetUserDefaultLocaleName);
  ok += EnvBoxAttach(&TrueGetSystemDefaultLocaleName,
                     HookGetSystemDefaultLocaleName);
  ok += EnvBoxAttach(&TrueGetUserDefaultLCID, HookGetUserDefaultLCID);
  ok += EnvBoxAttach(&TrueGetSystemDefaultLCID, HookGetSystemDefaultLCID);
  ok += EnvBoxAttach(&TrueGetLocaleInfoEx, HookGetLocaleInfoEx);
  ok += EnvBoxAttach(&TrueGetLocaleInfoW, HookGetLocaleInfoW);
  return ok;
}