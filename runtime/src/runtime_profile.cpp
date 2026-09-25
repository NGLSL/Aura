// Immutable Runtime Profile loaded once at DLL init.
//
// V0.3 config cutover (ticket 45):
//   1. Preferred: Runtime IPC Bootstrap (Named Pipe) - Broker/Host returns a
//      RuntimeProfile DTO (PROFILE message). No file I/O, no TOML in C++.
//   2. Fallback: ENVBOX_* structured value vars written by the Host into the
//      Environment Block (Win32 only). Still no file / TOML parsing.
//   3. Otherwise Startup Fail Policy - never run unvirtualized.
//
// C++ never reads profiles.toml and never validates Profile business rules.

#include "runtime_profile.h"

#include <stdio.h>
#include <string.h>

#include "ipc_bootstrap.h"

static RuntimeProfile g_profile = {};
static int g_loaded = 0;
static char g_dll_path_a[MAX_PATH] = {};

const RuntimeProfile* EnvBoxProfile() {
  return g_loaded ? &g_profile : nullptr;
}

const char* EnvBoxRuntimeDllPathA() {
  return g_dll_path_a[0] ? g_dll_path_a : nullptr;
}

static int ReadEnvW(const wchar_t* name, wchar_t* buf, DWORD cap) {
  DWORD n = GetEnvironmentVariableW(name, buf, cap);
  if (n == 0 || n >= cap) {
    buf[0] = L'\0';
    return 0;
  }
  return 1;
}

static int ReadEnvA(const char* name, char* buf, DWORD cap) {
  DWORD n = GetEnvironmentVariableA(name, buf, cap);
  if (n == 0 || n >= cap) {
    buf[0] = '\0';
    return 0;
  }
  return 1;
}

static void Utf8ToWide(const char* src, wchar_t* dst, size_t dst_cap) {
  MultiByteToWideChar(CP_UTF8, 0, src, -1, dst, (int)dst_cap);
  dst[dst_cap - 1] = L'\0';
}

int EnvBoxLookupTimeZone(const wchar_t* windows_id,
                         DYNAMIC_TIME_ZONE_INFORMATION* out) {
  if (!windows_id || !out) {
    return 0;
  }
  DWORD index = 0;
  for (;;) {
    DYNAMIC_TIME_ZONE_INFORMATION info = {};
    DWORD status = EnumDynamicTimeZoneInformation(index, &info);
    if (status != ERROR_SUCCESS) {
      break;
    }
    if (_wcsicmp(info.TimeZoneKeyName, windows_id) == 0) {
      *out = info;
      return 1;
    }
    index++;
    if (index > 1024) {
      break;
    }
  }
  return 0;
}

// Merge an IPC-fetched profile into g_profile (IPC payload is authoritative
// when used). Preserves fields the payload omitted only when it is empty.
static void ApplyIpcProfile(const RuntimeProfile* src) {
  if (src->profile_id[0] != L'\0') {
    wcsncpy_s(g_profile.profile_id, src->profile_id, _TRUNCATE);
  }
  if (src->instance_id[0] != L'\0') {
    wcsncpy_s(g_profile.instance_id, src->instance_id, _TRUNCATE);
  }
  if (src->has_locale) {
    wcsncpy_s(g_profile.locale_name, src->locale_name, _TRUNCATE);
    g_profile.has_locale = 1;
  }
  if (src->has_ui) {
    wcsncpy_s(g_profile.ui_language, src->ui_language, _TRUNCATE);
    g_profile.has_ui = 1;
  }
  if (src->has_region) {
    wcsncpy_s(g_profile.region, src->region, _TRUNCATE);
    g_profile.has_region = 1;
  }
  if (src->has_tz) {
    wcsncpy_s(g_profile.tz_windows, src->tz_windows, _TRUNCATE);
    g_profile.has_tz = 1;
  }
  if (src->tz_iana[0] != L'\0') {
    wcsncpy_s(g_profile.tz_iana, src->tz_iana, _TRUNCATE);
  }
  g_profile.inherit_children = src->inherit_children;
  g_profile.audit = src->audit;
  g_profile.dns_mode = src->dns_mode;
  g_profile.dns_server_count = src->dns_server_count;
  for (int i = 0; i < src->dns_server_count && i < ENVBOX_DNS_MAX; i++) {
    strncpy_s(g_profile.dns_servers[i], src->dns_servers[i], _TRUNCATE);
  }
  g_profile.registry_path_count = src->registry_path_count;
  for (int i = 0; i < src->registry_path_count && i < ENVBOX_REG_MAX; i++) {
    wcsncpy_s(g_profile.registry_paths[i], src->registry_paths[i], _TRUNCATE);
  }
}

// Split `a;b;c` into rows. Returns count, or -1 on overflow (caller Fail Open).
static int SplitListA(const char* raw, char out[][64], int max_items) {
  if (raw == nullptr || raw[0] == '\0') {
    return 0;
  }
  int count = 0;
  const char* p = raw;
  while (*p && count < max_items) {
    const char* semi = strchr(p, ';');
    size_t n = semi ? (size_t)(semi - p) : strlen(p);
    if (n > 0 && n < 64) {
      memcpy(out[count], p, n);
      out[count][n] = '\0';
      count++;
    }
    if (!semi) {
      break;
    }
    p = semi + 1;
  }
  if (*p && count >= max_items) {
    // Overflow: Fail Open (DNS) / drop extras (registry) - never silent lie.
    return -1;
  }
  return count;
}

static void SplitListW(const wchar_t* raw, wchar_t out[][128], int max_items,
                       int* out_count) {
  *out_count = 0;
  if (raw == nullptr || raw[0] == L'\0') {
    return;
  }
  const wchar_t* p = raw;
  while (*p && *out_count < max_items) {
    const wchar_t* semi = wcschr(p, L';');
    size_t n = semi ? (size_t)(semi - p) : wcslen(p);
    if (n > 0 && n < 128) {
      memcpy(out[*out_count], p, n * sizeof(wchar_t));
      out[*out_count][n] = L'\0';
      (*out_count)++;
    }
    if (!semi) {
      break;
    }
    p = semi + 1;
  }
}

// ENVBOX_* structured value fallback (no file, no TOML).
// Required: locale_name, ui_language, region, tz_windows.
static int LoadFromEnvValues() {
  wchar_t tmp[128];

  if (ReadEnvW(L"ENVBOX_LOCALE_NAME", g_profile.locale_name, 85)) {
    g_profile.has_locale = 1;
  }
  if (ReadEnvW(L"ENVBOX_UI_LANGUAGE", g_profile.ui_language, 85)) {
    g_profile.has_ui = 1;
  }
  if (ReadEnvW(L"ENVBOX_REGION", g_profile.region, 16)) {
    g_profile.has_region = 1;
  }
  if (ReadEnvW(L"ENVBOX_TZ_WINDOWS", g_profile.tz_windows, 128)) {
    g_profile.has_tz = 1;
  }
  ReadEnvW(L"ENVBOX_TZ_IANA", g_profile.tz_iana, 128);

  wchar_t dns_mode[8] = {};
  if (ReadEnvW(L"ENVBOX_DNS_MODE", dns_mode, 8) && dns_mode[0] == L'1') {
    g_profile.dns_mode = 1;
  }

  char dns_raw[512] = {};
  if (ReadEnvA("ENVBOX_DNS_SERVERS", dns_raw, (DWORD)sizeof(dns_raw))) {
    char rows[ENVBOX_DNS_MAX][64];
    int n = SplitListA(dns_raw, rows, ENVBOX_DNS_MAX);
    if (n < 0) {
      OutputDebugStringA("EnvBox: dns servers overflow, DNS View disabled\n");
      g_profile.dns_mode = 0;
      g_profile.dns_server_count = 0;
    } else {
      g_profile.dns_server_count = n;
      for (int i = 0; i < n; i++) {
        strncpy_s(g_profile.dns_servers[i], rows[i], _TRUNCATE);
      }
    }
  }
  if (g_profile.dns_mode == 1 && g_profile.dns_server_count == 0) {
    OutputDebugStringA("EnvBox: virtual_view without servers, DNS View disabled\n");
    g_profile.dns_mode = 0;
  }

  wchar_t reg_raw[2048] = {};
  if (ReadEnvW(L"ENVBOX_REGISTRY_PATHS", reg_raw,
               (DWORD)sizeof(reg_raw) / sizeof(wchar_t))) {
    SplitListW(reg_raw, g_profile.registry_paths, ENVBOX_REG_MAX,
               &g_profile.registry_path_count);
    (void)tmp;
  }

  if (!g_profile.has_locale || !g_profile.has_ui || !g_profile.has_region ||
      !g_profile.has_tz) {
    OutputDebugStringA("EnvBox: ENVBOX_* value fallback incomplete\n");
    return 0;
  }
  return 1;
}

int EnvBoxLoadProfile() {
  if (g_loaded) {
    return 1;  // immutable after first successful init
  }
  ReadEnvW(L"ENVBOX_PROFILE_ID", g_profile.profile_id, 64);
  ReadEnvW(L"ENVBOX_INSTANCE_ID", g_profile.instance_id, 64);

  // Child process inheritance default: on (Application may disable).
  wchar_t inherit[8] = {};
  if (ReadEnvW(L"ENVBOX_INHERIT_CHILDREN", inherit, 8) &&
      (inherit[0] == L'0' || inherit[0] == L'f' || inherit[0] == L'F')) {
    g_profile.inherit_children = 0;
  } else {
    g_profile.inherit_children = 1;
  }

  // Audit Mode default: off (ticket 20). Exact "1" enables.
  wchar_t audit[8] = {};
  g_profile.audit =
      (ReadEnvW(L"ENVBOX_AUDIT", audit, 8) && audit[0] == L'1' &&
       audit[1] == L'\0')
          ? 1
          : 0;

  // Remember this module's path for DetourUpdateProcessWithDll on children.
  HMODULE self = nullptr;
  if (GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                             GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                         reinterpret_cast<LPCWSTR>(&EnvBoxLoadProfile), &self)) {
    wchar_t wpath[MAX_PATH] = {};
    if (GetModuleFileNameW(self, wpath, MAX_PATH) > 0) {
      WideCharToMultiByte(CP_ACP, 0, wpath, -1, g_dll_path_a, MAX_PATH,
                          nullptr, nullptr);
    }
  }

  // Preferred: Broker/Host IPC PROFILE DTO (works for packaged roots too).
  int loaded = 0;
  {
    RuntimeProfile ipc = {};
    if (EnvBoxIpcFetchProfile(&ipc)) {
      ApplyIpcProfile(&ipc);
      loaded = 1;
    }
  }

  // Fallback: ENVBOX_* structured values (Win32 Environment Block). No TOML.
  if (!loaded) {
    if (LoadFromEnvValues()) {
      loaded = 1;
    }
  }

  if (!loaded) {
    // Startup Fail Policy: never run without a Profile (IPC or ENVBOX_*).
    return 0;
  }

  g_loaded = 1;
  // Best-effort readiness notice; never affects startup success.
  EnvBoxIpcNotifyRuntimeReady();
  return 1;
}
