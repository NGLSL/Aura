// Locale hooks (ticket 05+07). Keep user/system default locale names and LCIDs
// consistent with Profile. Fail Open on any error. Size queries return Profile
// length (never Host) so exact-alloc callers cannot silently see Host values.

#include "hooks.h"

#include <stdio.h>
#include <wchar.h>

#include "audit.h"

static int(WINAPI* TrueGetUserDefaultLocaleName)(LPWSTR, int) =
    GetUserDefaultLocaleName;
static int(WINAPI* TrueGetSystemDefaultLocaleName)(LPWSTR, int) =
    GetSystemDefaultLocaleName;
static LCID(WINAPI* TrueGetUserDefaultLCID)() = GetUserDefaultLCID;
static LCID(WINAPI* TrueGetSystemDefaultLCID)() = GetSystemDefaultLCID;
static int(WINAPI* TrueGetLocaleInfoEx)(LPCWSTR, LCTYPE, LPWSTR, int) =
    GetLocaleInfoEx;
static int(WINAPI* TrueGetLocaleInfoW)(LCID, LCTYPE, LPWSTR, int) = GetLocaleInfoW;
static int(WINAPI* TrueGetLocaleInfoA)(LCID, LCTYPE, LPSTR, int) = GetLocaleInfoA;

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
    EnvBoxAuditEventW("GetUserDefaultLocaleName", 1, pfl->locale_name);
    return CopyProfileLocaleName(pfl->locale_name, lpLocaleName, cchLocaleName);
  }
  EnvBoxAuditEvent("GetUserDefaultLocaleName", 0, nullptr);
  return TrueGetUserDefaultLocaleName(lpLocaleName, cchLocaleName);
}

static int WINAPI HookGetSystemDefaultLocaleName(LPWSTR lpLocaleName,
                                                 int cchLocaleName) {
  // Environment-consistent: same Profile locale as the user default.
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr && pfl->has_locale) {
    EnvBoxAuditEventW("GetSystemDefaultLocaleName", 1, pfl->locale_name);
    return CopyProfileLocaleName(pfl->locale_name, lpLocaleName, cchLocaleName);
  }
  EnvBoxAuditEvent("GetSystemDefaultLocaleName", 0, nullptr);
  return TrueGetSystemDefaultLocaleName(lpLocaleName, cchLocaleName);
}

static LCID ProfileLocaleLcid() {
  return EnvBoxProfileLcid();
}

static LCID WINAPI HookGetUserDefaultLCID() {
  LCID lcid = ProfileLocaleLcid();
  if (lcid != 0) {
    char buf[16];
    _snprintf_s(buf, _TRUNCATE, "0x%08x", (unsigned)lcid);
    EnvBoxAuditEvent("GetUserDefaultLCID", 1, buf);
    return lcid;
  }
  EnvBoxAuditEvent("GetUserDefaultLCID", 0, nullptr);
  return TrueGetUserDefaultLCID();
}

static LCID WINAPI HookGetSystemDefaultLCID() {
  LCID lcid = ProfileLocaleLcid();
  if (lcid != 0) {
    char buf[16];
    _snprintf_s(buf, _TRUNCATE, "0x%08x", (unsigned)lcid);
    EnvBoxAuditEvent("GetSystemDefaultLCID", 1, buf);
    return lcid;
  }
  EnvBoxAuditEvent("GetSystemDefaultLCID", 0, nullptr);
  return TrueGetSystemDefaultLCID();
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
  const RuntimeProfile* pfl = EnvBoxProfile();
  LCID v = VirtualizeLcid(Locale);
  if (pfl != nullptr && pfl->has_locale && v != Locale) {
    EnvBoxAuditEvent("GetLocaleInfoW", 1, "default-lcid-rewritten");
  } else {
    EnvBoxAuditEvent("GetLocaleInfoW", 0, nullptr);
  }
  return TrueGetLocaleInfoW(v, LCType, lpLCData, cchData);
}

static int WINAPI HookGetLocaleInfoA(LCID Locale, LCTYPE LCType, LPSTR lpLCData,
                                     int cchData) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  LCID v = VirtualizeLcid(Locale);
  if (pfl != nullptr && pfl->has_locale && v != Locale) {
    EnvBoxAuditEvent("GetLocaleInfoA", 1, "default-lcid-rewritten");
  } else {
    EnvBoxAuditEvent("GetLocaleInfoA", 0, nullptr);
  }
  return TrueGetLocaleInfoA(v, LCType, lpLCData, cchData);
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
  LPCWSTR v = VirtualizeLocaleName(lpLocaleName, pfl);
  if (v != lpLocaleName) {
    EnvBoxAuditEventW("GetLocaleInfoEx", 1, pfl ? pfl->locale_name : L"");
  } else {
    EnvBoxAuditEvent("GetLocaleInfoEx", 0, nullptr);
  }
  return TrueGetLocaleInfoEx(v, LCType, lpLCData, cchData);
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
  ok += EnvBoxAttach(&TrueGetLocaleInfoA, HookGetLocaleInfoA);
  return ok;
}
