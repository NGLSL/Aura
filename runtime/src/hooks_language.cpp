// UI Language hooks (ticket 05+07). Profile language first in preferred lists.
// Fail Open on any error. Size-query count matches post-dedupe write count.
// MUI_LANGUAGE_ID lists use hex LANGIDs (e.g. 0409), never locale names.

#include "hooks.h"

#include <stdio.h>

#include <string.h>
#include <wchar.h>

#include <string>
#include <vector>

#include "audit.h"

#ifndef MUI_LANGUAGE_ID
#define MUI_LANGUAGE_ID 0x4
#endif
#ifndef MUI_LANGUAGE_NAME
#define MUI_LANGUAGE_NAME 0x8
#endif

static LANGID(WINAPI* TrueGetUserDefaultUILanguage)() = GetUserDefaultUILanguage;
static LANGID(WINAPI* TrueGetSystemDefaultUILanguage)() =
    GetSystemDefaultUILanguage;
static BOOL(WINAPI* TrueGetUserPreferredUILanguages)(DWORD, PULONG, PZZWSTR,
                                                     PULONG) =
    GetUserPreferredUILanguages;
static BOOL(WINAPI* TrueGetSystemPreferredUILanguages)(DWORD, PULONG, PZZWSTR,
                                                       PULONG) =
    GetSystemPreferredUILanguages;
static BOOL(WINAPI* TrueGetThreadPreferredUILanguages)(DWORD, PULONG, PZZWSTR,
                                                       PULONG) =
    GetThreadPreferredUILanguages;
static BOOL(WINAPI* TrueGetProcessPreferredUILanguages)(DWORD, PULONG, PZZWSTR,
                                                        PULONG) =
    GetProcessPreferredUILanguages;

static LANGID ProfileUiLangId() {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_ui) {
    return 0;
  }
  LCID lcid = LocaleNameToLCID(pfl->ui_language, 0);
  if (lcid == 0) {
    return 0;
  }
  return LANGIDFROMLCID(lcid);
}

static LANGID WINAPI HookGetUserDefaultUILanguage() {
  LANGID id = ProfileUiLangId();
  if (id != 0) {
    char buf[16];
    _snprintf_s(buf, _TRUNCATE, "0x%04x", (unsigned)id);
    EnvBoxAuditEvent("GetUserDefaultUILanguage", 1, buf);
    return id;
  }
  EnvBoxAuditEvent("GetUserDefaultUILanguage", 0, nullptr);
  return TrueGetUserDefaultUILanguage();
}

static LANGID WINAPI HookGetSystemDefaultUILanguage() {
  // Environment-consistent with user UI language.
  LANGID id = ProfileUiLangId();
  if (id != 0) {
    char buf[16];
    _snprintf_s(buf, _TRUNCATE, "0x%04x", (unsigned)id);
    EnvBoxAuditEvent("GetSystemDefaultUILanguage", 1, buf);
    return id;
  }
  EnvBoxAuditEvent("GetSystemDefaultUILanguage", 0, nullptr);
  return TrueGetSystemDefaultUILanguage();
}

typedef BOOL(WINAPI* PreferredFn)(DWORD, PULONG, PZZWSTR, PULONG);

static std::wstring ProfileUiToken(DWORD dwFlags, bool* ok) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  *ok = true;
  if ((dwFlags & MUI_LANGUAGE_ID) != 0) {
    LANGID id = ProfileUiLangId();
    if (id == 0) {
      *ok = false;
      return std::wstring();
    }
    wchar_t buf[8];
    _snwprintf_s(buf, _TRUNCATE, L"%04x", (unsigned)id);
    return std::wstring(buf);
  }
  return std::wstring(pfl->ui_language);
}

// Profile token first, then unique host items (same form as dwFlags requests).
static BOOL WINAPI HookPreferredUILanguages(const char* api_name, PreferredFn true_fn,
                                            DWORD dwFlags, PULONG pulNumLanguages,
                                            PZZWSTR pwszLanguagesBuffer,
                                            PULONG pcchLanguagesBuffer) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_ui || pulNumLanguages == nullptr ||
      pcchLanguagesBuffer == nullptr) {
    EnvBoxAuditEvent(api_name, 0, nullptr);
    return true_fn(dwFlags, pulNumLanguages, pwszLanguagesBuffer,
                   pcchLanguagesBuffer);
  }

  std::vector<wchar_t> host;
  ULONG host_size = 0;
  ULONG host_count = 0;
  true_fn(dwFlags, &host_count, nullptr, &host_size);
  if (host_size > 0) {
    host.resize(host_size);
    ULONG size2 = host_size;
    if (!true_fn(dwFlags, &host_count, host.data(), &size2)) {
      EnvBoxAuditEvent(api_name, 0, "fail-open");
      return true_fn(dwFlags, pulNumLanguages, pwszLanguagesBuffer,
                     pcchLanguagesBuffer);
    }
  }

  bool token_ok = false;
  std::wstring head = ProfileUiToken(dwFlags, &token_ok);
  if (!token_ok) {
    EnvBoxAuditEvent(api_name, 0, "fail-open");
    return true_fn(dwFlags, pulNumLanguages, pwszLanguagesBuffer,
                   pcchLanguagesBuffer);
  }
  std::vector<std::wstring> items;
  items.push_back(head);
  if (!host.empty()) {
    const wchar_t* p = host.data();
    while (*p) {
      std::wstring entry(p);
      p += entry.size() + 1;
      if (_wcsicmp(entry.c_str(), head.c_str()) != 0) {
        items.push_back(entry);
      }
    }
  }

  size_t total = 1;
  for (const auto& s : items) {
    total += s.size() + 1;
  }

  // Size query: count is post-dedupe items.size(), not host_count+1.
  if (pwszLanguagesBuffer == nullptr) {
    *pulNumLanguages = (ULONG)items.size();
    *pcchLanguagesBuffer = (ULONG)total;
    EnvBoxAuditEventW(api_name, 1, head.c_str());
    return TRUE;
  }
  if (*pcchLanguagesBuffer < total) {
    *pulNumLanguages = (ULONG)items.size();
    *pcchLanguagesBuffer = (ULONG)total;
    EnvBoxAuditEvent(api_name, 1, "insufficient-buffer");
    SetLastError(ERROR_INSUFFICIENT_BUFFER);
    return FALSE;
  }
  wchar_t* out = pwszLanguagesBuffer;
  size_t remaining = *pcchLanguagesBuffer;
  for (const auto& s : items) {
    if (remaining <= s.size()) {
      *pcchLanguagesBuffer = (ULONG)total;
      EnvBoxAuditEvent(api_name, 1, "insufficient-buffer");
      SetLastError(ERROR_INSUFFICIENT_BUFFER);
      return FALSE;
    }
    wcscpy_s(out, remaining, s.c_str());
    out += s.size() + 1;
    remaining -= s.size() + 1;
  }
  *out = L'\0';
  *pulNumLanguages = (ULONG)items.size();
  *pcchLanguagesBuffer = (ULONG)total;
  EnvBoxAuditEventW(api_name, 1, head.c_str());
  return TRUE;
}

static BOOL WINAPI HookGetUserPreferredUILanguages(DWORD dwFlags,
                                                   PULONG pulNumLanguages,
                                                   PZZWSTR pwszLanguagesBuffer,
                                                   PULONG pcchLanguagesBuffer) {
  return HookPreferredUILanguages("GetUserPreferredUILanguages",
                                  TrueGetUserPreferredUILanguages, dwFlags,
                                  pulNumLanguages, pwszLanguagesBuffer,
                                  pcchLanguagesBuffer);
}

static BOOL WINAPI HookGetSystemPreferredUILanguages(DWORD dwFlags,
                                                     PULONG pulNumLanguages,
                                                     PZZWSTR pwszLanguagesBuffer,
                                                     PULONG pcchLanguagesBuffer) {
  return HookPreferredUILanguages("GetSystemPreferredUILanguages",
                                  TrueGetSystemPreferredUILanguages, dwFlags,
                                  pulNumLanguages, pwszLanguagesBuffer,
                                  pcchLanguagesBuffer);
}

static BOOL WINAPI HookGetThreadPreferredUILanguages(DWORD dwFlags,
                                                     PULONG pulNumLanguages,
                                                     PZZWSTR pwszLanguagesBuffer,
                                                     PULONG pcchLanguagesBuffer) {
  return HookPreferredUILanguages("GetThreadPreferredUILanguages",
                                  TrueGetThreadPreferredUILanguages, dwFlags,
                                  pulNumLanguages, pwszLanguagesBuffer,
                                  pcchLanguagesBuffer);
}

static BOOL WINAPI HookGetProcessPreferredUILanguages(
    DWORD dwFlags, PULONG pulNumLanguages, PZZWSTR pwszLanguagesBuffer,
    PULONG pcchLanguagesBuffer) {
  return HookPreferredUILanguages("GetProcessPreferredUILanguages",
                                  TrueGetProcessPreferredUILanguages, dwFlags,
                                  pulNumLanguages, pwszLanguagesBuffer,
                                  pcchLanguagesBuffer);
}

int EnvBoxInstallLanguageHooks() {
  int ok = 0;
  ok += EnvBoxAttach(&TrueGetUserDefaultUILanguage, HookGetUserDefaultUILanguage);
  ok += EnvBoxAttach(&TrueGetSystemDefaultUILanguage,
                     HookGetSystemDefaultUILanguage);
  ok += EnvBoxAttach(&TrueGetUserPreferredUILanguages,
                     HookGetUserPreferredUILanguages);
  ok += EnvBoxAttach(&TrueGetSystemPreferredUILanguages,
                     HookGetSystemPreferredUILanguages);
  ok += EnvBoxAttach(&TrueGetThreadPreferredUILanguages,
                     HookGetThreadPreferredUILanguages);
  ok += EnvBoxAttach(&TrueGetProcessPreferredUILanguages,
                     HookGetProcessPreferredUILanguages);
  return ok;
}