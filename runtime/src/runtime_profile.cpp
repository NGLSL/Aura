#include "runtime_profile.h"

#include <stdio.h>
#include <string.h>

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

// Minimal TOML string value extract: key = "value" at line start within block.
static int ExtractTomlString(const char* data, size_t len, const char* key,
                            char* out, size_t out_cap) {
  size_t klen = strlen(key);
  size_t i = 0;
  while (i + klen < len) {
    // Anchor to line start (or after newline).
    int at_line_start = (i == 0) || (data[i - 1] == '\n');
    if (at_line_start && strncmp(data + i, key, klen) == 0) {
      size_t j = i + klen;
      while (j < len && (data[j] == ' ' || data[j] == '\t')) j++;
      if (j < len && data[j] == '=') {
        j++;
        while (j < len && (data[j] == ' ' || data[j] == '\t')) j++;
        if (j < len && (data[j] == '"' || data[j] == '\'')) {
          char q = data[j++];
          size_t o = 0;
          while (j < len && data[j] != q && o + 1 < out_cap) {
            if (data[j] == '\\' && j + 1 < len) {
              j++;
            }
            out[o++] = data[j++];
          }
          out[o] = '\0';
          return o > 0;
        }
      }
    }
    i++;
  }
  return 0;
}

static void Utf8ToWide(const char* src, wchar_t* dst, size_t dst_cap) {
  MultiByteToWideChar(CP_UTF8, 0, src, -1, dst, (int)dst_cap);
  dst[dst_cap - 1] = L'\0';
}

// Parse `key = [ "a", "b", ... ]` (single or multi-line) into char rows.
// Returns count, or -1 when the array exceeds max_items (caller must Fail Open).
static int ExtractTomlStringArray(const char* data, size_t len, const char* key,
                                 char out[][64], int max_items,
                                 size_t item_cap) {
  size_t klen = strlen(key);
  size_t i = 0;
  while (i + klen < len) {
    int at_line_start = (i == 0) || (data[i - 1] == '\n');
    if (at_line_start && strncmp(data + i, key, klen) == 0) {
      size_t j = i + klen;
      while (j < len && (data[j] == ' ' || data[j] == '\t')) j++;
      if (j < len && data[j] == '=') {
        j++;
        while (j < len && data[j] != '[') j++;
        if (j >= len) return 0;
        j++;  // skip '['
        int count = 0;
        while (j < len && data[j] != ']') {
          while (j < len && (data[j] == ' ' || data[j] == '\t' || data[j] == '\n' ||
                             data[j] == '\r' || data[j] == ',')) {
            j++;
          }
          if (j < len && (data[j] == '"' || data[j] == '\'')) {
            if (count >= max_items) {
              return -1;  // overflow -> Fail Open (never silent truncate)
            }
            char q = data[j++];
            size_t o = 0;
            while (j < len && data[j] != q && o + 1 < item_cap) {
              out[count][o++] = data[j++];
            }
            if (j < len && data[j] == q) j++;
            out[count][o] = '\0';
            if (o > 0) count++;
          } else if (j < len && data[j] != ']') {
            j++;
          }
        }
        return count;
      }
    }
    i++;
  }
  return 0;
}

// Scope to a TOML table header like "[profiles.dns]" through the next table.
static int FindTomlTable(const char* data, size_t len, const char* header,
                         size_t* out_start, size_t* out_end) {
  size_t hlen = strlen(header);
  const char* hit = strstr(data, header);
  if (!hit) {
    return 0;
  }
  size_t start = (size_t)(hit - data) + hlen;
  size_t end = len;
  for (size_t j = start; j < len; j++) {
    if (data[j] == '[' && (j == 0 || data[j - 1] == '\n')) {
      end = j;
      break;
    }
  }
  *out_start = start;
  *out_end = end;
  return 1;
}

// Find [[profiles]] block whose id = "<profile_id>".
static int FindProfileBlock(const char* data, size_t len, const char* profile_id,
                            size_t* out_start, size_t* out_end) {
  char needle[96];
  _snprintf_s(needle, sizeof(needle), _TRUNCATE, "id = \"%s\"", profile_id);
  // Also accept single-quoted TOML form.
  char needle2[96];
  _snprintf_s(needle2, sizeof(needle2), _TRUNCATE, "id = '%s'", profile_id);

  const char* hit = strstr(data, needle);
  if (!hit) {
    hit = strstr(data, needle2);
  }
  if (!hit) {
    return 0;
  }
  size_t pos = (size_t)(hit - data);

  size_t start = 0;
  for (size_t i = 0; i < pos; i++) {
    if (i + 12 <= len && strncmp(data + i, "[[profiles]]", 12) == 0) {
      start = i;
    }
  }
  size_t end = len;
  for (size_t j = pos; j + 12 <= len; j++) {
    if (strncmp(data + j, "[[profiles]]", 12) == 0 && j > start) {
      end = j;
      break;
    }
  }
  *out_start = start;
  *out_end = end;
  return 1;
}

// Load from profiles.toml. Returns 1 only when the profile block is found and
// all four virtualized fields are populated. Missing store or missing profile
// is fatal (Startup Fail Policy) - never silent unvirtualized launch.
static int LoadFromProfilesToml(const char* profile_id_utf8) {
  char root[MAX_PATH];
  DWORD n = GetEnvironmentVariableA("ENVBOX_CONFIG_ROOT", root, (DWORD)sizeof(root));
  if (n == 0 || n >= sizeof(root)) {
    // Match ConfigStore::default_root(): %LOCALAPPDATA%\EnvBox
    char appdata[MAX_PATH] = {};
    if (GetEnvironmentVariableA("LOCALAPPDATA", appdata, (DWORD)sizeof(appdata)) == 0) {
      OutputDebugStringA("EnvBox: no ENVBOX_CONFIG_ROOT or LOCALAPPDATA\n");
      return 0;
    }
    _snprintf_s(root, sizeof(root), _TRUNCATE, "%s\\EnvBox", appdata);
  }

  char path[MAX_PATH + 32];
  _snprintf_s(path, sizeof(path), _TRUNCATE, "%s\\profiles.toml", root);

  HANDLE file = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, nullptr,
                            OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    DWORD err = GetLastError();
    char msg[256];
    _snprintf_s(msg, sizeof(msg), _TRUNCATE,
                "EnvBox: profiles.toml open failed (GetLastError=%lu)\n", err);
    OutputDebugStringA(msg);
    return 0;  // Startup Fail Policy: do not silently run unvirtualized
  }

  static char data[65536];
  DWORD read = 0;
  BOOL ok = ReadFile(file, data, (DWORD)sizeof(data) - 1, &read, nullptr);
  DWORD read_err = ok ? 0 : GetLastError();
  CloseHandle(file);
  if (!ok) {
    char msg[128];
    _snprintf_s(msg, sizeof(msg), _TRUNCATE,
                "EnvBox: profiles.toml read failed (GetLastError=%lu)\n", read_err);
    OutputDebugStringA(msg);
    return 0;
  }
  data[read] = '\0';

  size_t start = 0, end = 0;
  if (!FindProfileBlock(data, read, profile_id_utf8, &start, &end)) {
    OutputDebugStringA("EnvBox: profile id not found in profiles.toml\n");
    return 0;
  }
  const char* block = data + start;
  size_t blen = end - start;

  char tmp[256];
  if (ExtractTomlString(block, blen, "locale_name", tmp, sizeof(tmp))) {
    Utf8ToWide(tmp, g_profile.locale_name, 85);
    g_profile.has_locale = 1;
  }
  if (ExtractTomlString(block, blen, "ui_language", tmp, sizeof(tmp))) {
    Utf8ToWide(tmp, g_profile.ui_language, 85);
    g_profile.has_ui = 1;
  }
  if (ExtractTomlString(block, blen, "region", tmp, sizeof(tmp))) {
    Utf8ToWide(tmp, g_profile.region, 16);
    g_profile.has_region = 1;
  }
  if (ExtractTomlString(block, blen, "windows_id", tmp, sizeof(tmp))) {
    Utf8ToWide(tmp, g_profile.tz_windows, 128);
    g_profile.has_tz = 1;
  }
  if (ExtractTomlString(block, blen, "iana_id", tmp, sizeof(tmp))) {
    Utf8ToWide(tmp, g_profile.tz_iana, 128);
  }

  // DNS View (ticket 08). Scope: [profiles.dns] only. Optional -> Host.
  {
    size_t ds = 0, de = 0;
    if (FindTomlTable(block, blen, "[profiles.dns]", &ds, &de)) {
      const char* dblock = block + ds;
      size_t dlen = de - ds;
      if (ExtractTomlString(dblock, dlen, "mode", tmp, sizeof(tmp))) {
        if (_stricmp(tmp, "virtual_view") == 0 ||
            _stricmp(tmp, "virtualview") == 0) {
          g_profile.dns_mode = 1;
        }
      }
      int n = ExtractTomlStringArray(dblock, dlen, "servers",
                                    g_profile.dns_servers, ENVBOX_DNS_MAX, 64);
      if (n < 0) {
        // Overflow: Fail Open to Host for both DNS APIs (no silent truncate).
        OutputDebugStringA("EnvBox: dns servers overflow, DNS View disabled\n");
        g_profile.dns_mode = 0;
        g_profile.dns_server_count = 0;
      } else {
        g_profile.dns_server_count = n;
        if (g_profile.dns_mode == 1 && n == 0) {
          OutputDebugStringA("EnvBox: virtual_view without servers, DNS View disabled\n");
          g_profile.dns_mode = 0;
        }
      }
    }
  }

  // Registry Virtual View (ticket 09): optional extra whitelist_paths.
  {
    size_t rs = 0, re_ = 0;
    if (FindTomlTable(block, blen, "[profiles.registry]", &rs, &re_)) {
      const char* rblock = block + rs;
      size_t rlen = re_ - rs;
      char paths[ENVBOX_REG_MAX][64];
      int n = ExtractTomlStringArray(rblock, rlen, "whitelist_paths", paths,
                                     ENVBOX_REG_MAX, 64);
      if (n < 0) {
        OutputDebugStringA("EnvBox: registry whitelist overflow\n");
        n = 0;
      }
      for (int i = 0; i < n; i++) {
        if (g_profile.registry_path_count >= ENVBOX_REG_MAX) {
          break;
        }
        Utf8ToWide(paths[i],
                   g_profile.registry_paths[g_profile.registry_path_count], 128);
        g_profile.registry_path_count++;
      }
    }
  }

  if (!g_profile.has_locale || !g_profile.has_ui || !g_profile.has_region ||
      !g_profile.has_tz) {
    OutputDebugStringA("EnvBox: profile block incomplete (need locale/ui/region/tz)\n");
    return 0;
  }
  return 1;
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

int EnvBoxLoadProfile() {
  if (g_loaded) {
    return 1;  // immutable after first successful init
  }
  if (!ReadEnvW(L"ENVBOX_PROFILE_ID", g_profile.profile_id, 64)) {
    OutputDebugStringA("EnvBox: ENVBOX_PROFILE_ID missing\n");
    return 0;
  }
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

  char id_utf8[64] = {};
  WideCharToMultiByte(CP_UTF8, 0, g_profile.profile_id, -1, id_utf8,
                      (int)sizeof(id_utf8), nullptr, nullptr);
  if (!LoadFromProfilesToml(id_utf8)) {
    return 0;
  }

  g_loaded = 1;
  return 1;
}
