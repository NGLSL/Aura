// UI Language hooks (ticket 05+07). Profile language first in preferred lists.
// Fail Open on any error. Size-query count matches post-dedupe write count.
// MUI_LANGUAGE_ID lists use hex LANGIDs (e.g. 0409), never locale names.

#include "hooks.h"

#include <stdio.h>

#include <string.h>
#include <wchar.h>
#include <winstring.h>

#include <atomic>
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
  static const LANGID id = LANGIDFROMLCID(LocaleNameToLCID(pfl->ui_language, 0));
  return id;
}

static LANGID WINAPI HookGetUserDefaultUILanguage() {
  LANGID id = ProfileUiLangId();
  if (id != 0) {
    if (EnvBoxAuditEnabled()) {
      char buf[16];
      _snprintf_s(buf, _TRUNCATE, "0x%04x", (unsigned)id);
      EnvBoxAuditEvent("GetUserDefaultUILanguage", 1, buf);
    }
    return id;
  }
  EnvBoxAuditEvent("GetUserDefaultUILanguage", 0, nullptr);
  return TrueGetUserDefaultUILanguage();
}

static LANGID WINAPI HookGetSystemDefaultUILanguage() {
  // Environment-consistent with user UI language.
  LANGID id = ProfileUiLangId();
  if (id != 0) {
    if (EnvBoxAuditEnabled()) {
      char buf[16];
      _snprintf_s(buf, _TRUNCATE, "0x%04x", (unsigned)id);
      EnvBoxAuditEvent("GetSystemDefaultUILanguage", 1, buf);
    }
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

// The International PowerShell module resolves these account-language
// readers from bcp47langs.dll at run time, after Runtime startup.
using UserLanguagesFn = HRESULT(WINAPI*)(wchar_t, HSTRING*);
using UserInputMethodsFn = HRESULT(WINAPI*)(PCWSTR, wchar_t, HSTRING*);
using DefaultInputMethodFn = HRESULT(WINAPI*)(HSTRING, HSTRING*);

static FARPROC(WINAPI* TrueGetProcAddress)(HMODULE, LPCSTR) = GetProcAddress;
static std::atomic<UserLanguagesFn> g_user_languages{nullptr};
static std::atomic<UserInputMethodsFn> g_user_input_methods{nullptr};

static HRESULT WINAPI HookGetUserLanguages(wchar_t separator, HSTRING* languages) {
  const DWORD saved_error = GetLastError();
  const wchar_t* language = EnvBoxProfile()->ui_language;
  if (languages != nullptr &&
      SUCCEEDED(WindowsCreateString(language, static_cast<UINT32>(wcslen(language)),
                                    languages))) {
    EnvBoxAuditEventW("GetUserLanguages", 1, language);
    SetLastError(saved_error);
    return S_OK;
  }
  SetLastError(saved_error);
  return g_user_languages.load(std::memory_order_acquire)(separator, languages);
}

static HRESULT GetDefaultInputMethod(PCWSTR language, HSTRING* result) {
  HSTRING_HEADER header{};
  HSTRING tag = nullptr;
  HRESULT hr = WindowsCreateStringReference(
      language, static_cast<UINT32>(wcslen(language)), &header, &tag);
  if (FAILED(hr)) return hr;

  // This is the same Windows default used when constructing a new language
  // entry. It also supplies the default IME TIP for scripts such as Japanese.
  HMODULE input = LoadLibraryExW(L"ext-ms-win-globalization-input-l1-1-2.dll",
                                nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
  if (input == nullptr) return HRESULT_FROM_WIN32(GetLastError());
  const auto get_default = reinterpret_cast<DefaultInputMethodFn>(
      TrueGetProcAddress(input, "WGIGetDefaultInputMethodForLanguage"));
  hr = get_default != nullptr ? get_default(tag, result) : E_NOTIMPL;
  FreeLibrary(input);
  return hr;
}

static HRESULT WINAPI HookGetUserLanguageInputMethods(PCWSTR language,
                                                      wchar_t separator,
                                                      HSTRING* methods) {
  const DWORD saved_error = GetLastError();
  const wchar_t* profile_language = EnvBoxProfile()->ui_language;
  if (methods != nullptr && language != nullptr &&
      _wcsicmp(language, profile_language) == 0) {
    HSTRING defaults = nullptr;
    if (SUCCEEDED(GetDefaultInputMethod(profile_language, &defaults))) {
      *methods = defaults;
      EnvBoxAuditEventW("GetUserLanguageInputMethods", 1, profile_language);
      SetLastError(saved_error);
      return S_OK;
    }
    WindowsDeleteString(defaults);
  }
  SetLastError(saved_error);
  return g_user_input_methods.load(std::memory_order_acquire)(language, separator,
                                                             methods);
}

static bool IsWindowsLanguageModule(HMODULE module) {
  wchar_t path[MAX_PATH];
  const DWORD length = GetModuleFileNameW(module, path, ARRAYSIZE(path));
  if (length == 0 || length >= ARRAYSIZE(path)) return false;
  wchar_t* name = wcsrchr(path, L'\\');
  if (name == nullptr || _wcsicmp(name + 1, L"bcp47langs.dll") != 0) return false;
  *name = L'\0';

  wchar_t system_directory[MAX_PATH];
  UINT size = GetSystemDirectoryW(system_directory, ARRAYSIZE(system_directory));
  if (size > 0 && size < ARRAYSIZE(system_directory) &&
      _wcsicmp(path, system_directory) == 0) {
    return true;
  }
#ifndef _WIN64
  size = GetSystemWow64DirectoryW(system_directory, ARRAYSIZE(system_directory));
  return size > 0 && size < ARRAYSIZE(system_directory) &&
         _wcsicmp(path, system_directory) == 0;
#else
  return false;
#endif
}

static FARPROC WINAPI HookGetProcAddress(HMODULE module, LPCSTR name) {
  FARPROC address = TrueGetProcAddress(module, name);
  if (address == nullptr || IS_INTRESOURCE(name)) return address;
  const bool languages = strcmp(name, "GetUserLanguages") == 0;
  if (!languages && strcmp(name, "GetUserLanguageInputMethods") != 0) {
    return address;
  }
  const DWORD saved_error = GetLastError();
  if (IsWindowsLanguageModule(module)) {
    if (languages) {
      g_user_languages.store(reinterpret_cast<UserLanguagesFn>(address),
                             std::memory_order_release);
      address = reinterpret_cast<FARPROC>(HookGetUserLanguages);
    } else {
      g_user_input_methods.store(reinterpret_cast<UserInputMethodsFn>(address),
                                 std::memory_order_release);
      address = reinterpret_cast<FARPROC>(HookGetUserLanguageInputMethods);
    }
  }
  SetLastError(saved_error);
  return address;
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

int EnvBoxInstallUserLanguageHooks() {
  if (!EnvBoxProfile()->has_ui || DetourTransactionBegin() != NO_ERROR) return 0;
  if (DetourUpdateThread(GetCurrentThread()) != NO_ERROR ||
      !EnvBoxAttach(&TrueGetProcAddress, HookGetProcAddress)) {
    DetourTransactionAbort();
    return 0;
  }
  return DetourTransactionCommit() == NO_ERROR;
}
