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

#include <string>

#include "ipc_bootstrap.h"

static RuntimeProfile g_profile = {};
static int g_loaded = 0;
static int g_environment_complete = 0;
static char g_dll_path_a[MAX_PATH] = {};

const RuntimeProfile* EnvBoxProfile() {
  return g_loaded ? &g_profile : nullptr;
}

int EnvBoxProfileEnvironmentComplete() {
  return g_loaded && g_environment_complete;
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
  // Hot path: GetTimeZoneInformation hooks call this every time. EnumDynamic-
  // TimeZoneInformation walks the whole host table (100+ rows), which made
  // Chrome-style workloads ~300x slower. Profile tz is process-immutable, so
  // cache the first successful lookup (and the miss for the same id).
  static DYNAMIC_TIME_ZONE_INFORMATION s_cache = {};
  static int s_cached = 0;
  static int s_found = 0;
  static wchar_t s_id[128] = {0};
  if (s_cached && _wcsicmp(s_id, windows_id) == 0) {
    if (s_found) {
      *out = s_cache;
    }
    return s_found;
  }
  DWORD index = 0;
  int found = 0;
  DYNAMIC_TIME_ZONE_INFORMATION hit = {};
  for (;;) {
    DYNAMIC_TIME_ZONE_INFORMATION info = {};
    DWORD status = EnumDynamicTimeZoneInformation(index, &info);
    if (status != ERROR_SUCCESS) {
      break;
    }
    if (_wcsicmp(info.TimeZoneKeyName, windows_id) == 0) {
      hit = info;
      found = 1;
      break;
    }
    index++;
    if (index > 1024) {
      break;
    }
  }
  wcsncpy_s(s_id, windows_id, _TRUNCATE);
  s_cache = hit;
  s_found = found;
  s_cached = 1;
  if (found) {
    *out = hit;
  }
  return found;
}

LCID EnvBoxProfileLcid() {
  // Hot path: IME / Chrome call GetUserDefaultLCID and registry Locale value
  // in tight loops while typing. Locale is process-immutable - cache the
  // LocaleNameToLCID result once.
  static LCID s_lcid = 0;
  static int s_cached = 0;
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_locale) {
    return 0;
  }
  if (!s_cached) {
    s_lcid = LocaleNameToLCID(pfl->locale_name, 0);
    s_cached = 1;
  }
  return s_lcid;
}

// ---------------------------------------------------------------------------
// WebRTC Privacy policy tokens (ticket 54). Mirrors envbox-core::WebRtcPolicy
// parse / as_str / chromium_ip_handling_policy exactly; C++ never invents new
// semantics. Canonical tokens: host, public_interface_only, proxy_only, strict.
// ---------------------------------------------------------------------------
static const wchar_t kWebRtcHost[] = L"host";
static const wchar_t kWebRtcPublicInterfaceOnly[] = L"public_interface_only";
static const wchar_t kWebRtcProxyOnly[] = L"proxy_only";
static const wchar_t kWebRtcStrict[] = L"strict";

const wchar_t* EnvBoxWebRtcChromiumValue(const wchar_t* policy_token) {
  if (policy_token == nullptr || policy_token[0] == L'\0') {
    return nullptr;
  }
  if (_wcsicmp(policy_token, kWebRtcPublicInterfaceOnly) == 0) {
    return L"default_public_interface_only";
  }
  if (_wcsicmp(policy_token, kWebRtcProxyOnly) == 0 ||
      _wcsicmp(policy_token, kWebRtcStrict) == 0) {
    return L"disable_non_proxied_udp";
  }
  return nullptr;  // host (and unknown): never add / never overwrite.
}

int EnvBoxNormalizeWebRtcPolicy(const wchar_t* raw, wchar_t* out, size_t cap) {
  if (out == nullptr || cap == 0) {
    return 0;
  }
  wcsncpy_s(out, cap, kWebRtcHost, _TRUNCATE);
  if (raw == nullptr) {
    return 1;  // empty -> host
  }
  while (*raw == L' ' || *raw == L'\t' || *raw == L'\n' || *raw == L'\r') {
    raw++;
  }
  size_t len = wcslen(raw);
  while (len > 0 && (raw[len - 1] == L' ' || raw[len - 1] == L'\t' ||
                     raw[len - 1] == L'\n' || raw[len - 1] == L'\r')) {
    len--;
  }
  if (len == 0) {
    return 1;  // empty -> host
  }
  // Bounded ASCII lowercase (mirror Rust to_ascii_lowercase + alias match).
  wchar_t norm[32];
  if (len >= sizeof(norm) / sizeof(norm[0])) {
    return 0;
  }
  for (size_t i = 0; i < len; i++) {
    wchar_t c = raw[i];
    if (c >= L'A' && c <= L'Z') {
      c = (wchar_t)(c - L'A' + L'a');
    }
    norm[i] = c;
  }
  norm[len] = L'\0';

  const wchar_t* canonical = nullptr;
  if (wcscmp(norm, kWebRtcHost) == 0) {
    canonical = kWebRtcHost;
  } else if (wcscmp(norm, kWebRtcPublicInterfaceOnly) == 0 ||
             wcscmp(norm, L"public-interface-only") == 0) {
    canonical = kWebRtcPublicInterfaceOnly;
  } else if (wcscmp(norm, kWebRtcProxyOnly) == 0 ||
             wcscmp(norm, L"proxy-only") == 0 ||
             wcscmp(norm, L"disable_non_proxied_udp") == 0) {
    canonical = kWebRtcProxyOnly;
  } else if (wcscmp(norm, kWebRtcStrict) == 0) {
    canonical = kWebRtcStrict;
  }
  if (canonical == nullptr) {
    return 0;  // unknown: never silently run as host.
  }
  wcsncpy_s(out, cap, canonical, _TRUNCATE);
  return 1;
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
  g_profile.dns_config_version = src->dns_config_version;
  g_profile.dns_strict = src->dns_strict;
  g_profile.dns_upstream_count = src->dns_upstream_count;
  for (int i = 0; i < src->dns_upstream_count && i < ENVBOX_DNS_MAX; ++i)
    g_profile.dns_upstreams[i] = src->dns_upstreams[i];
  g_profile.dns_server_count = src->dns_server_count;
  for (int i = 0; i < src->dns_server_count && i < ENVBOX_DNS_MAX; i++) {
    strncpy_s(g_profile.dns_servers[i], src->dns_servers[i], _TRUNCATE);
  }
  g_profile.registry_path_count = src->registry_path_count;
  for (int i = 0; i < src->registry_path_count && i < ENVBOX_REG_MAX; i++) {
    wcsncpy_s(g_profile.registry_paths[i], src->registry_paths[i], _TRUNCATE);
  }
  g_profile.environment_count = src->environment_count;
  for (int i = 0; i < src->environment_count && i < ENVBOX_ENV_MAX; i++) {
    wcsncpy_s(g_profile.environment[i], src->environment[i], _TRUNCATE);
  }
  if (src->webrtc_policy[0] != L'\0') {
    wcsncpy_s(g_profile.webrtc_policy, src->webrtc_policy, _TRUNCATE);
  }
}

// Apply Profile environment values to this process after IPC bootstrap. This
// is required for packaged roots because AUMID activation cannot receive a
// custom Environment Block. Internal ENVBOX_* identity always wins, matching
// the Rust Win32 environment builder.
static void ApplyCurrentProcessEnvironment(const RuntimeProfile* profile,
                                           int environment_complete) {
  if (profile == nullptr) {
    return;
  }
  // A packaged root has no custom CreateProcess environment block. With a
  // complete IPC Profile, remove inherited host POSIX locale values before
  // applying explicit Profile entries. ENVBOX_* fallback lacks the Profile
  // override list, so keep its already-merged inherited environment intact.
  LPWCH block = environment_complete ? GetEnvironmentStringsW() : nullptr;
  if (block != nullptr) {
    for (const wchar_t* entry = block; *entry != L'\0';
         entry += wcslen(entry) + 1) {
      const wchar_t* eq = wcschr(entry, L'=');
      if (eq == nullptr || eq == entry) {
        continue;
      }
      const size_t key_len = (size_t)(eq - entry);
      if ((key_len == 4 && _wcsnicmp(entry, L"LANG", 4) == 0) ||
          (key_len == 8 && _wcsnicmp(entry, L"LANGUAGE", 8) == 0) ||
          (key_len >= 3 && _wcsnicmp(entry, L"LC_", 3) == 0)) {
        std::wstring key(entry, key_len);
        SetEnvironmentVariableW(key.c_str(), nullptr);
      }
    }
    FreeEnvironmentStringsW(block);
  }
  for (int i = 0; i < profile->environment_count && i < ENVBOX_ENV_MAX; i++) {
    const wchar_t* entry = profile->environment[i];
    const wchar_t* eq = wcschr(entry, L'=');
    if (eq == nullptr || eq == entry) {
      continue;
    }
    std::wstring key(entry, (size_t)(eq - entry));
    if (_wcsnicmp(key.c_str(), L"ENVBOX_", 7) == 0) {
      continue;
    }
    SetEnvironmentVariableW(key.c_str(), eq + 1);
  }

  SetEnvironmentVariableW(L"ENVBOX_PROFILE_ID", profile->profile_id);
  SetEnvironmentVariableW(L"ENVBOX_INSTANCE_ID", profile->instance_id);
  SetEnvironmentVariableW(L"ENVBOX_INHERIT_CHILDREN",
                          profile->inherit_children ? L"1" : L"0");
  SetEnvironmentVariableW(L"ENVBOX_AUDIT", profile->audit ? L"1" : L"0");

  // Persist the complete structured fallback in the packaged root. Windows
  // package activation cannot receive Aura's custom Environment Block. Once
  // the Broker exits, descendants can still bootstrap from these inherited
  // values and keep the same immutable Profile.
  SetEnvironmentVariableW(L"ENVBOX_LOCALE_NAME", profile->locale_name);
  SetEnvironmentVariableW(L"ENVBOX_UI_LANGUAGE", profile->ui_language);
  SetEnvironmentVariableW(L"ENVBOX_REGION", profile->region);
  SetEnvironmentVariableW(L"ENVBOX_TZ_WINDOWS", profile->tz_windows);
  SetEnvironmentVariableW(L"ENVBOX_TZ_IANA", profile->tz_iana);
  SetEnvironmentVariableW(L"ENVBOX_DNS_MODE",
                          profile->dns_mode ? L"1" : L"0");

  std::wstring dns_servers;
  for (int i = 0; i < profile->dns_server_count && i < ENVBOX_DNS_MAX; i++) {
    wchar_t server[64] = {};
    if (MultiByteToWideChar(CP_UTF8, 0, profile->dns_servers[i], -1, server,
                            64) <= 0) {
      continue;
    }
    if (!dns_servers.empty()) {
      dns_servers.push_back(L';');
    }
    dns_servers.append(server);
  }
  SetEnvironmentVariableW(L"ENVBOX_DNS_SERVERS", dns_servers.c_str());
  if (profile->dns_config_version == 1) {
    EnvBoxEmitDnsConfiguration(profile, [](void*, const char* key, const char* value) {
      wchar_t name[128] = L"ENVBOX_";
      size_t n = strlen(key);
      if (n + 8 > ARRAYSIZE(name)) return 0;
      for (size_t i = 0; i < n; ++i) {
        char c = key[i];
        name[i + 7] = c >= 'a' && c <= 'z' ? c - 'a' + 'A' : c;
      }
      wchar_t text[2048];
      if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value, -1, text, ARRAYSIZE(text))) return 0;
      return SetEnvironmentVariableW(name, text) ? 1 : 0;
    }, nullptr);
  }

  std::wstring registry_paths;
  for (int i = 0;
       i < profile->registry_path_count && i < ENVBOX_REG_MAX; i++) {
    if (!registry_paths.empty()) {
      registry_paths.push_back(L';');
    }
    registry_paths.append(profile->registry_paths[i]);
  }
  SetEnvironmentVariableW(L"ENVBOX_REGISTRY_PATHS", registry_paths.c_str());
  SetEnvironmentVariableW(L"ENVBOX_WEBRTC_POLICY", profile->webrtc_policy);
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
static int DnsEnvironmentField(void*, const char* key, char* value, size_t capacity) {
  wchar_t name[128] = L"ENVBOX_";
  size_t length = strlen(key);
  if (length + 8 > ARRAYSIZE(name) || capacity > 2048) return -1;
  for (size_t i = 0; i < length; ++i) {
    char c = key[i];
    name[i + 7] = c >= 'a' && c <= 'z' ? c - 'a' + 'A' : c;
  }
  wchar_t buffer[2048];
  SetLastError(ERROR_SUCCESS);
  DWORD count = GetEnvironmentVariableW(name, buffer, ARRAYSIZE(buffer));
  if (count == 0) {
    if (GetLastError() == ERROR_ENVVAR_NOT_FOUND) return 0;
    value[0] = '\0';
    return 1;
  }
  if (count >= ARRAYSIZE(buffer)) return -1;
  return WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, buffer, -1, value,
                             static_cast<int>(capacity), nullptr, nullptr) > 0 ? 1 : -1;
}

// Unlike IPC's enumerable key map, the getter alone cannot detect extra fields.
// Check the process environment against the exact snapshot shape after decode;
// a missing version must not turn residual typed fields into a legacy profile.
static int DnsEnvironmentShapeValid(const RuntimeProfile* profile) {
  struct Names { wchar_t values[128][128]; int count; } names = {};
  if (!EnvBoxEmitDnsConfiguration(profile, [](void* context, const char* key, const char*) {
    auto names = static_cast<Names*>(context);
    size_t length = strlen(key);
    if (names->count >= 128 || length + 8 > 128) return 0;
    wchar_t* name = names->values[names->count++];
    wcscpy_s(name, 128, L"ENVBOX_");
    for (size_t i = 0; i < length; ++i)
      name[i + 7] = key[i] >= 'a' && key[i] <= 'z' ? key[i] - 'a' + 'A' : key[i];
    return 1;
  }, &names)) return 0;
  LPWCH block = GetEnvironmentStringsW();
  if (!block) return 0;
  int valid = 1;
  for (const wchar_t* entry = block; *entry && valid; entry += wcslen(entry) + 1) {
    if (_wcsnicmp(entry, L"ENVBOX_DNS_", 11) != 0) continue;
    const wchar_t* separator = wcschr(entry, L'=');
    if (!separator) { valid = 0; break; }
    size_t length = separator - entry;
    // The address projection is a compatibility view, not a typed authority.
    // UDP_PORT remains a legacy-only fixture knob and cannot override v1 ports.
    bool allowed = (length == 18 && _wcsnicmp(entry, L"ENVBOX_DNS_SERVERS", length) == 0) ||
                   (length == 19 && _wcsnicmp(entry, L"ENVBOX_DNS_UDP_PORT", length) == 0);
    for (int i = 0; i < names.count && !allowed; ++i)
      allowed = wcslen(names.values[i]) == length && _wcsnicmp(entry, names.values[i], length) == 0;
    if (!allowed) valid = 0;
  }
  FreeEnvironmentStringsW(block);
  return valid;
}

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

  if (!EnvBoxDecodeDnsConfiguration(&g_profile, DnsEnvironmentField, nullptr) ||
      !DnsEnvironmentShapeValid(&g_profile)) {
    OutputDebugStringA("EnvBox: invalid/incomplete DNS snapshot\n");
    return 0;
  }
  // An empty VirtualView stays virtual and resolves unsuccessfully in strict
  // mode. It must not expose the host's resolver configuration.

  wchar_t reg_raw[2048] = {};
  if (ReadEnvW(L"ENVBOX_REGISTRY_PATHS", reg_raw,
               (DWORD)sizeof(reg_raw) / sizeof(wchar_t))) {
    SplitListW(reg_raw, g_profile.registry_paths, ENVBOX_REG_MAX,
               &g_profile.registry_path_count);
    (void)tmp;
  }

  // Browser / Network Guard WebRTC policy token (ticket 51/54). Default
  // host; unknown tokens are rejected (never silently run with the wrong
  // policy). Alias set matches envbox-core::WebRtcPolicy::parse.
  wchar_t webrtc_raw[32] = {};
  if (ReadEnvW(L"ENVBOX_WEBRTC_POLICY", webrtc_raw, 32)) {
    if (!EnvBoxNormalizeWebRtcPolicy(webrtc_raw, g_profile.webrtc_policy, 32)) {
      OutputDebugStringA("EnvBox: invalid ENVBOX_WEBRTC_POLICY token\n");
      return 0;
    }
  } else {
    wcsncpy_s(g_profile.webrtc_policy, L"host", _TRUNCATE);
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
      g_environment_complete = 1;
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
  if (g_profile.profile_id[0] == L'\0' || g_profile.instance_id[0] == L'\0') {
    OutputDebugStringA("EnvBox: Profile identity missing\n");
    return 0;
  }

  ApplyCurrentProcessEnvironment(&g_profile, g_environment_complete);
  g_loaded = 1;
  return 1;
}
