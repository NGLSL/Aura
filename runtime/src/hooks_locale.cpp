// Locale hooks (ticket 05+07). Keep user/system default locale names and LCIDs
// consistent with Profile. Fail Open on any error. Size queries return Profile
// length (never Host) so exact-alloc callers cannot silently see Host values.

#include "hooks.h"

#include <atomic>
#include <limits.h>
#include <stdio.h>
#include <wchar.h>

#include "audit.h"

static int(WINAPI* TrueGetUserDefaultLocaleName)(LPWSTR, int) =
    GetUserDefaultLocaleName;
static int(WINAPI* TrueGetSystemDefaultLocaleName)(LPWSTR, int) =
    GetSystemDefaultLocaleName;
static LCID(WINAPI* TrueGetUserDefaultLCID)() = GetUserDefaultLCID;
static LCID(WINAPI* TrueGetSystemDefaultLCID)() = GetSystemDefaultLCID;
static LANGID(WINAPI* TrueGetUserDefaultLangID)() = GetUserDefaultLangID;
static LANGID(WINAPI* TrueGetSystemDefaultLangID)() = GetSystemDefaultLangID;
static LCID(WINAPI* TrueGetThreadLocale)() = GetThreadLocale;
static int(WINAPI* TrueGetLocaleInfoEx)(LPCWSTR, LCTYPE, LPWSTR, int) =
    GetLocaleInfoEx;
static int(WINAPI* TrueGetLocaleInfoW)(LCID, LCTYPE, LPWSTR, int) = GetLocaleInfoW;
static int(WINAPI* TrueGetLocaleInfoA)(LCID, LCTYPE, LPSTR, int) = GetLocaleInfoA;
static UINT(WINAPI* TrueGetACP)() = GetACP;
static UINT(WINAPI* TrueGetOEMCP)() = GetOEMCP;
static int(WINAPI* TrueMultiByteToWideChar)(UINT, DWORD, LPCCH, int, LPWSTR,
                                             int) = MultiByteToWideChar;
static int(WINAPI* TrueWideCharToMultiByte)(UINT, DWORD, LPCWCH, int, LPSTR, int,
                                             LPCCH, LPBOOL) = WideCharToMultiByte;
// GetACP/GetOEMCP must never advertise a Profile page unless both conversion
// entry points are detoured too.  A partial Detours transaction is otherwise
// an inconsistent view for ANSI callers.
static std::atomic<int> g_code_page_hooks_ready{0};

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

static LANGID WINAPI HookGetUserDefaultLangID() {
  DWORD last_error = GetLastError();
  LCID lcid = ProfileLocaleLcid();
  if (lcid != 0) {
    LANGID id = LANGIDFROMLCID(lcid);
    char value[16];
    _snprintf_s(value, _TRUNCATE, "0x%04x", (unsigned)id);
    EnvBoxAuditEvent("GetUserDefaultLangID", 1, value);
    SetLastError(last_error);
    return id;
  }
  EnvBoxAuditEvent("GetUserDefaultLangID", 0, nullptr);
  SetLastError(last_error);
  return TrueGetUserDefaultLangID();
}

static LANGID WINAPI HookGetSystemDefaultLangID() {
  DWORD last_error = GetLastError();
  LCID lcid = ProfileLocaleLcid();
  if (lcid != 0) {
    LANGID id = LANGIDFROMLCID(lcid);
    char value[16];
    _snprintf_s(value, _TRUNCATE, "0x%04x", (unsigned)id);
    EnvBoxAuditEvent("GetSystemDefaultLangID", 1, value);
    SetLastError(last_error);
    return id;
  }
  EnvBoxAuditEvent("GetSystemDefaultLangID", 0, nullptr);
  SetLastError(last_error);
  return TrueGetSystemDefaultLangID();
}

static LCID WINAPI HookGetThreadLocale() {
  DWORD last_error = GetLastError();
  LCID lcid = ProfileLocaleLcid();
  LCID actual = TrueGetThreadLocale();
  if (actual == 0) {
    return actual;
  }
  // SetThreadLocale is an explicit per-thread choice. Only substitute the
  // inherited host default, preserving a caller-selected thread locale.
  if (lcid != 0 && actual == TrueGetUserDefaultLCID()) {
    char value[16];
    _snprintf_s(value, _TRUNCATE, "0x%08x", (unsigned)lcid);
    EnvBoxAuditEvent("GetThreadLocale", 1, value);
    SetLastError(last_error);
    return lcid;
  }
  EnvBoxAuditEvent("GetThreadLocale", 0, nullptr);
  SetLastError(last_error);
  return actual;
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

// A Profile currently identifies its locale, rather than storing an
// independent code-page override.  Derive the ANSI/OEM pages from that
// immutable locale using the NLS data Windows already owns.  This keeps
// GetACP/GetOEMCP and calls that pass CP_ACP/CP_OEMCP on the same view.  A
// missing or unavailable page fails open to the host API; the Runtime never
// changes the process or host system locale.
static UINT ProfileCodePage(LCTYPE type) {
  struct RestoreLastError {
    DWORD saved = GetLastError();
    ~RestoreLastError() { SetLastError(saved); }
  } restore;
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_locale || pfl->locale_name[0] == L'\0') {
    return 0;
  }

  wchar_t text[16] = {};
  int n = TrueGetLocaleInfoEx(pfl->locale_name, type, text,
                              (int)(sizeof(text) / sizeof(text[0])));
  if (n <= 1) {
    return 0;
  }
  wchar_t* end = nullptr;
  unsigned long value = wcstoul(text, &end, 10);
  if (end == text || *end != L'\0' || value == 0 || value > UINT_MAX) {
    return 0;
  }
  UINT code_page = (UINT)value;
  return IsValidCodePage(code_page) ? code_page : 0;
}

static UINT ProfileAnsiCodePage() {
  // The Profile is immutable and all hooks are installed only after the
  // Profile is loaded.  This cache avoids querying NLS on every conversion.
  static std::atomic<UINT> cached{UINT_MAX};
  UINT value = cached.load(std::memory_order_relaxed);
  if (value == UINT_MAX) {
    DWORD last_error = GetLastError();
    UINT process_page = TrueGetACP();
    SetLastError(last_error);
    // An application's activeCodePage=UTF-8 manifest is an explicit process
    // choice; keep its actual ACP and CP_ACP conversion semantics intact.
    value = process_page == CP_UTF8
                ? 0
                : ProfileCodePage(LOCALE_IDEFAULTANSICODEPAGE);
    cached.store(value, std::memory_order_relaxed);
  }
  return value;
}

static UINT ProfileOemCodePage() {
  static std::atomic<UINT> cached{UINT_MAX};
  UINT value = cached.load(std::memory_order_relaxed);
  if (value == UINT_MAX) {
    value = ProfileCodePage(LOCALE_IDEFAULTCODEPAGE);
    cached.store(value, std::memory_order_relaxed);
  }
  return value;
}

static UINT VirtualizeCodePage(UINT code_page) {
  if (!g_code_page_hooks_ready.load(std::memory_order_acquire)) {
    return code_page;
  }
  if (code_page == CP_ACP) {
    UINT profile = ProfileAnsiCodePage();
    return profile != 0 ? profile : code_page;
  }
  if (code_page == CP_OEMCP) {
    UINT profile = ProfileOemCodePage();
    return profile != 0 ? profile : code_page;
  }
  if (code_page == CP_THREAD_ACP) {
    DWORD last_error = GetLastError();
    bool is_default_thread = TrueGetThreadLocale() == TrueGetUserDefaultLCID();
    SetLastError(last_error);
    if (!is_default_thread) {
      return code_page;
    }
    // CP_THREAD_ACP follows the thread locale. An explicitly selected
    // SetThreadLocale value continues through Windows unchanged.
    UINT profile = ProfileAnsiCodePage();
    return profile != 0 ? profile : code_page;
  }
  return code_page;
}

static UINT WINAPI HookGetACP() {
  DWORD last_error = GetLastError();
  if (!g_code_page_hooks_ready.load(std::memory_order_acquire)) {
    return TrueGetACP();
  }
  UINT profile = ProfileAnsiCodePage();
  if (profile != 0) {
    char value[16];
    _snprintf_s(value, _TRUNCATE, "%u", profile);
    EnvBoxAuditEvent("GetACP", 1, value);
    SetLastError(last_error);
    return profile;
  }
  EnvBoxAuditEvent("GetACP", 0, nullptr);
  SetLastError(last_error);
  return TrueGetACP();
}

static UINT WINAPI HookGetOEMCP() {
  DWORD last_error = GetLastError();
  if (!g_code_page_hooks_ready.load(std::memory_order_acquire)) {
    return TrueGetOEMCP();
  }
  UINT profile = ProfileOemCodePage();
  if (profile != 0) {
    char value[16];
    _snprintf_s(value, _TRUNCATE, "%u", profile);
    EnvBoxAuditEvent("GetOEMCP", 1, value);
    SetLastError(last_error);
    return profile;
  }
  EnvBoxAuditEvent("GetOEMCP", 0, nullptr);
  SetLastError(last_error);
  return TrueGetOEMCP();
}

static int WINAPI HookMultiByteToWideChar(UINT code_page, DWORD flags,
                                          LPCCH source, int source_size,
                                          LPWSTR destination,
                                          int destination_size) {
  UINT routed = VirtualizeCodePage(code_page);
  return TrueMultiByteToWideChar(routed, flags, source, source_size,
                                destination, destination_size);
}

static int WINAPI HookWideCharToMultiByte(UINT code_page, DWORD flags,
                                          LPCWCH source, int source_size,
                                          LPSTR destination,
                                          int destination_size,
                                          LPCCH default_char,
                                          LPBOOL used_default_char) {
  UINT routed = VirtualizeCodePage(code_page);
  return TrueWideCharToMultiByte(routed, flags, source, source_size,
                                destination, destination_size, default_char,
                                used_default_char);
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
  ok += EnvBoxAttach(&TrueGetUserDefaultLangID, HookGetUserDefaultLangID);
  ok += EnvBoxAttach(&TrueGetSystemDefaultLangID, HookGetSystemDefaultLangID);
  ok += EnvBoxAttach(&TrueGetThreadLocale, HookGetThreadLocale);
  ok += EnvBoxAttach(&TrueGetLocaleInfoEx, HookGetLocaleInfoEx);
  ok += EnvBoxAttach(&TrueGetLocaleInfoW, HookGetLocaleInfoW);
  ok += EnvBoxAttach(&TrueGetLocaleInfoA, HookGetLocaleInfoA);
  int acp_ok = EnvBoxAttach(&TrueGetACP, HookGetACP);
  int oem_ok = EnvBoxAttach(&TrueGetOEMCP, HookGetOEMCP);
  int mb_to_wide_ok = EnvBoxAttach(&TrueMultiByteToWideChar,
                                   HookMultiByteToWideChar);
  int wide_to_mb_ok = EnvBoxAttach(&TrueWideCharToMultiByte,
                                   HookWideCharToMultiByte);
  g_code_page_hooks_ready.store(
      acp_ok && oem_ok && mb_to_wide_ok && wide_to_mb_ok,
      std::memory_order_release);
  ok += acp_ok + oem_ok + mb_to_wide_ok + wide_to_mb_ok;
  return ok;
}
