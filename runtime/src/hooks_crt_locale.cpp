// UCRT locale defaults.
//
// The Universal CRT resolves setlocale(LC_*, "") from the host Windows
// locale.  That bypasses the Win32 NLS hooks in hooks_locale.cpp, which leaves
// applications such as Python with a mixed Profile/Host view.  Keep this
// seam deliberately narrow: only the non-null empty-locale request is
// rewritten.  Queries and explicit locale names retain the CRT's behavior.

#include "hooks.h"

#include <locale.h>
#include <wchar.h>

#include "audit.h"

namespace {

using SetLocaleFn = char*(__cdecl*)(int, const char*);
using WSetLocaleFn = wchar_t*(__cdecl*)(int, const wchar_t*);

static SetLocaleFn TrueSetLocale = nullptr;
static WSetLocaleFn TrueWSetLocale = nullptr;

// Profile locale names are capped at 84 characters, but an application can
// provide a longer environment value.  Keep the hook bounded and fail open
// rather than allocating in an injected call path.
constexpr size_t kLocaleValueCap = 1024;
constexpr size_t kLocaleTextCap = 4096;

enum class ValueStatus {
  Missing,
  Present,
  Invalid,
};

// Read one current-process environment value.  Reading at call time is
// intentional: applications may change LC_* or LANG in their own process
// after Runtime load. Child creation reapplies the immutable Profile.
static ValueStatus ReadEnvironmentValue(const wchar_t* name, wchar_t* out,
                                        size_t cap) {
  if (name == nullptr || out == nullptr || cap == 0) {
    return ValueStatus::Invalid;
  }
  out[0] = L'\0';
  DWORD n = GetEnvironmentVariableW(name, out, (DWORD)cap);
  if (n == 0) {
    return ValueStatus::Missing;
  }
  if ((size_t)n >= cap) {
    out[0] = L'\0';
    return ValueStatus::Invalid;
  }
  return ValueStatus::Present;
}

static const wchar_t* CategoryEnvironmentName(int category) {
  switch (category) {
    case LC_COLLATE:
      return L"LC_COLLATE";
    case LC_CTYPE:
      return L"LC_CTYPE";
    case LC_MONETARY:
      return L"LC_MONETARY";
    case LC_NUMERIC:
      return L"LC_NUMERIC";
    case LC_TIME:
      return L"LC_TIME";
#ifdef LC_MESSAGES
    case LC_MESSAGES:
      return L"LC_MESSAGES";
#endif
    default:
      return nullptr;
  }
}

static bool CopyLocaleValue(const wchar_t* source, wchar_t* out, size_t cap) {
  if (source == nullptr || out == nullptr || cap == 0) {
    return false;
  }
  size_t n = wcslen(source);
  if (n == 0 || n >= cap) {
    return false;
  }
  wmemcpy(out, source, n + 1);
  return true;
}

static bool AppendLocalePart(wchar_t* out, size_t cap, size_t* length,
                             const wchar_t* name, const wchar_t* value) {
  if (out == nullptr || length == nullptr || name == nullptr || value == nullptr) {
    return false;
  }
  size_t name_len = wcslen(name);
  size_t value_len = wcslen(value);
  size_t separator = *length == 0 ? 0 : 1;
  if (*length > cap || name_len > cap - *length - separator ||
      value_len > cap - *length - separator - name_len - 1) {
    return false;
  }
  if (separator != 0) {
    out[(*length)++] = L';';
  }
  wmemcpy(out + *length, name, name_len);
  *length += name_len;
  out[(*length)++] = L'=';
  wmemcpy(out + *length, value, value_len);
  *length += value_len;
  out[*length] = L'\0';
  return true;
}

// Resolve the locale string for an empty setlocale request.  The result is
// either a simple locale name or UCRT's LC_CATEGORY=value;... form for
// LC_ALL when category-specific overrides are present.
static bool ResolveProfileFallback(wchar_t* out, size_t cap) {
  wchar_t value[kLocaleValueCap] = {};
  ValueStatus lang =
      ReadEnvironmentValue(L"LANG", value, sizeof(value) / sizeof(value[0]));
  if (lang == ValueStatus::Invalid) {
    return false;
  }
  if (lang == ValueStatus::Present) {
    return CopyLocaleValue(value, out, cap);
  }

  const RuntimeProfile* profile = EnvBoxProfile();
  if (profile == nullptr || !profile->has_locale) {
    return false;
  }
  return CopyLocaleValue(profile->locale_name, out, cap);
}

static bool ResolveEmptyLocale(int category, wchar_t* out, size_t cap) {
  if (out == nullptr || cap == 0) {
    return false;
  }
  out[0] = L'\0';

  wchar_t value[kLocaleValueCap] = {};
  ValueStatus all = ReadEnvironmentValue(
      L"LC_ALL", value, sizeof(value) / sizeof(value[0]));
  if (all == ValueStatus::Invalid) {
    return false;
  }
  if (all == ValueStatus::Present) {
    return CopyLocaleValue(value, out, cap);
  }

  const wchar_t* category_name = CategoryEnvironmentName(category);
  if (category != LC_ALL && category_name != nullptr) {
    ValueStatus category_value = ReadEnvironmentValue(
        category_name, value, sizeof(value) / sizeof(value[0]));
    if (category_value == ValueStatus::Invalid) {
      return false;
    }
    if (category_value == ValueStatus::Present) {
      return CopyLocaleValue(value, out, cap);
    }
    return ResolveProfileFallback(out, cap);
  }

  // LC_ALL has no single category-specific value.  When any category
  // override exists, construct the UCRT composite form and fill unspecified
  // categories from LANG, then Profile locale.
  static const wchar_t* const kCategories[] = {
      L"LC_COLLATE", L"LC_CTYPE", L"LC_MONETARY", L"LC_NUMERIC",
      L"LC_TIME",
#ifdef LC_MESSAGES
      L"LC_MESSAGES",
#endif
  };
  static const int kCategoryIds[] = {
      LC_COLLATE, LC_CTYPE, LC_MONETARY, LC_NUMERIC, LC_TIME,
#ifdef LC_MESSAGES
      LC_MESSAGES,
#endif
  };
  constexpr size_t kCategoryCount = sizeof(kCategoryIds) / sizeof(kCategoryIds[0]);

  wchar_t base[kLocaleValueCap] = {};
  if (!ResolveProfileFallback(base, sizeof(base) / sizeof(base[0]))) {
    return false;
  }

  bool has_category_override = false;
  ValueStatus statuses[kCategoryCount] = {};
  wchar_t values[kCategoryCount][kLocaleValueCap] = {};
  for (size_t i = 0; i < kCategoryCount; ++i) {
    statuses[i] = ReadEnvironmentValue(
        kCategories[i], values[i], sizeof(values[i]) / sizeof(values[i][0]));
    if (statuses[i] == ValueStatus::Invalid) {
      return false;
    }
    has_category_override |= statuses[i] == ValueStatus::Present;
  }
  if (!has_category_override) {
    return CopyLocaleValue(base, out, cap);
  }

  size_t length = 0;
  for (size_t i = 0; i < kCategoryCount; ++i) {
    const wchar_t* selected =
        statuses[i] == ValueStatus::Present ? values[i] : base;
    if (!AppendLocalePart(out, cap, &length, kCategories[i], selected)) {
      return false;
    }
  }
  return true;
}

static bool WideToUtf8(const wchar_t* source, char* out, int cap) {
  if (source == nullptr || out == nullptr || cap <= 0) {
    return false;
  }
  int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, source, -1, out,
                              cap, nullptr, nullptr);
  return n > 0;
}

static char* __cdecl HookSetLocale(int category, const char* locale) {
  if (TrueSetLocale == nullptr || locale == nullptr || locale[0] != '\0') {
    char* result = TrueSetLocale == nullptr ? nullptr : TrueSetLocale(category, locale);
    EnvBoxAuditEvent("setlocale", 0, locale == nullptr ? "query" : "explicit");
    return result;
  }

  wchar_t resolved[kLocaleTextCap] = {};
  char narrow[kLocaleTextCap] = {};
  if (ResolveEmptyLocale(category, resolved,
                         sizeof(resolved) / sizeof(resolved[0])) &&
      WideToUtf8(resolved, narrow, (int)sizeof(narrow))) {
    char* result = TrueSetLocale(category, narrow);
    if (result != nullptr) {
      EnvBoxAuditEvent("setlocale", 1, "empty-profile");
      return result;
    }
  }
  // Invalid or unsupported Profile/environment locale: preserve the CRT's
  // original host-resolution behavior instead of forcing a partial view.
  char* result = TrueSetLocale(category, locale);
  EnvBoxAuditEvent("setlocale", 0, "fail-open-empty");
  return result;
}

static wchar_t* __cdecl HookWSetLocale(int category, const wchar_t* locale) {
  if (TrueWSetLocale == nullptr || locale == nullptr || locale[0] != L'\0') {
    wchar_t* result =
        TrueWSetLocale == nullptr ? nullptr : TrueWSetLocale(category, locale);
    EnvBoxAuditEvent("_wsetlocale", 0, locale == nullptr ? "query" : "explicit");
    return result;
  }

  wchar_t resolved[kLocaleTextCap] = {};
  if (ResolveEmptyLocale(category, resolved,
                         sizeof(resolved) / sizeof(resolved[0]))) {
    wchar_t* result = TrueWSetLocale(category, resolved);
    if (result != nullptr) {
      EnvBoxAuditEvent("_wsetlocale", 1, "empty-profile");
      return result;
    }
  }
  wchar_t* result = TrueWSetLocale(category, locale);
  EnvBoxAuditEvent("_wsetlocale", 0, "fail-open-empty");
  return result;
}

}  // namespace

int EnvBoxInstallCrtLocaleHooks() {
  // Do not LoadLibrary from DllMain.  Most dynamically-linked UCRT clients,
  // including Python, already have ucrtbase.dll loaded before Runtime
  // injection.  If they do not, this optional hook fails open for that module.
  HMODULE ucrt = GetModuleHandleW(L"ucrtbase.dll");
  if (ucrt == nullptr) {
    return 0;
  }

  TrueSetLocale = reinterpret_cast<SetLocaleFn>(
      GetProcAddress(ucrt, "setlocale"));
  TrueWSetLocale = reinterpret_cast<WSetLocaleFn>(
      GetProcAddress(ucrt, "_wsetlocale"));

  int ok = 0;
  if (TrueSetLocale != nullptr) {
    ok += EnvBoxAttach(&TrueSetLocale, HookSetLocale);
  }
  if (TrueWSetLocale != nullptr) {
    ok += EnvBoxAttach(&TrueWSetLocale, HookWSetLocale);
  }
  return ok;
}
