// Registry Virtual View (ticket 09). Whitelist-only read virtualization for
// internationalization/timezone keys. NOT a Registry Sandbox: everything
// outside the whitelist passes through unchanged. Writes always pass through.
// Fail Open on any error.
//
// Every successful RegOpenKeyExW is tracked (not only whitelist hits) so
// relative/nested opens resolve to a full path later. Map is lock-protected.

#include <windows.h>

#include <stdio.h>
#include <string.h>

#include "hooks.h"
#include "runtime_profile.h"

#include "audit.h"

#ifndef ENVBOX_REG_TRACK
#define ENVBOX_REG_TRACK 512
#endif

static LSTATUS(WINAPI* TrueRegOpenKeyExW)(HKEY, LPCWSTR, DWORD, REGSAM,
                                         PHKEY) = RegOpenKeyExW;
static LSTATUS(WINAPI* TrueRegQueryValueExW)(HKEY, LPCWSTR, LPDWORD, LPDWORD,
                                             LPBYTE, LPDWORD) = RegQueryValueExW;
static LSTATUS(WINAPI* TrueRegGetValueW)(HKEY, LPCWSTR, LPCWSTR, DWORD,
                                         LPDWORD, PVOID, LPDWORD) = RegGetValueW;
static LSTATUS(WINAPI* TrueRegCloseKey)(HKEY) = RegCloseKey;

static const wchar_t* kIntlPath = L"HKEY_CURRENT_USER\\Control Panel\\International";
static const wchar_t* kTzPath =
    L"HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation";

struct TrackedKey {
  HKEY handle;
  wchar_t path[260];
};

static CRITICAL_SECTION g_lock;
static int g_lock_ready = 0;
static TrackedKey g_keys[ENVBOX_REG_TRACK];

static void EnsureLock() {
  if (!g_lock_ready) {
    InitializeCriticalSection(&g_lock);
    g_lock_ready = 1;
  }
}

static int PathEqualsOrUnder(const wchar_t* path, const wchar_t* root) {
  if (path == nullptr || root == nullptr) {
    return 0;
  }
  size_t rl = wcslen(root);
  if (_wcsnicmp(path, root, rl) != 0) {
    return 0;
  }
  return path[rl] == L'\0' || path[rl] == L'\\';
}

static int IsWhitelisted(const wchar_t* path) {
  if (path == nullptr || path[0] == L'\0') {
    return 0;
  }
  if (PathEqualsOrUnder(path, kIntlPath) || PathEqualsOrUnder(path, kTzPath)) {
    return 1;
  }
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr) {
    for (int i = 0; i < pfl->registry_path_count; i++) {
      if (PathEqualsOrUnder(path, pfl->registry_paths[i])) {
        return 1;
      }
    }
  }
  return 0;
}

static void RootLabel(HKEY k, wchar_t* out, size_t cap) {
  if (k == HKEY_CURRENT_USER) {
    wcsncpy_s(out, cap, L"HKEY_CURRENT_USER", _TRUNCATE);
  } else if (k == HKEY_LOCAL_MACHINE) {
    wcsncpy_s(out, cap, L"HKEY_LOCAL_MACHINE", _TRUNCATE);
  } else if (k == HKEY_CLASSES_ROOT) {
    wcsncpy_s(out, cap, L"HKEY_CLASSES_ROOT", _TRUNCATE);
  } else if (k == HKEY_USERS) {
    wcsncpy_s(out, cap, L"HKEY_USERS", _TRUNCATE);
  } else if (k == HKEY_CURRENT_CONFIG) {
    wcsncpy_s(out, cap, L"HKEY_CURRENT_CONFIG", _TRUNCATE);
  } else {
    out[0] = L'\0';
  }
}

// Copy tracked path (or empty). Caller owns out[]. Always lock-held or after copy.
static int LookupPath(HKEY hk, wchar_t* out, size_t cap) {
  EnsureLock();
  EnterCriticalSection(&g_lock);
  int found = 0;
  for (int i = 0; i < ENVBOX_REG_TRACK; i++) {
    if (g_keys[i].handle == hk) {
      wcsncpy_s(out, cap, g_keys[i].path, _TRUNCATE);
      found = 1;
      break;
    }
  }
  LeaveCriticalSection(&g_lock);
  return found;
}

static void TrackKey(HKEY hk, const wchar_t* path) {
  if (hk == nullptr || path == nullptr || path[0] == L'\0') {
    return;
  }
  EnsureLock();
  EnterCriticalSection(&g_lock);
  // Reuse slot if handle recycled; else first free; else evict slot 0.
  int slot = -1;
  for (int i = 0; i < ENVBOX_REG_TRACK; i++) {
    if (g_keys[i].handle == hk) {
      slot = i;
      break;
    }
    if (slot < 0 && g_keys[i].handle == nullptr) {
      slot = i;
    }
  }
  if (slot < 0) {
    slot = 0;
  }
  g_keys[slot].handle = hk;
  wcsncpy_s(g_keys[slot].path, path, _TRUNCATE);
  LeaveCriticalSection(&g_lock);
}

static void UntrackKey(HKEY hk) {
  EnsureLock();
  EnterCriticalSection(&g_lock);
  for (int i = 0; i < ENVBOX_REG_TRACK; i++) {
    if (g_keys[i].handle == hk) {
      g_keys[i].handle = nullptr;
      g_keys[i].path[0] = L'\0';
      break;
    }
  }
  LeaveCriticalSection(&g_lock);
}

// Full path = parent path (tracked or predefined root) + relative subkey.
static int JoinPath(HKEY parent, LPCWSTR subkey, wchar_t* out, size_t cap) {
  wchar_t base[260];
  if (!LookupPath(parent, base, 260)) {
    RootLabel(parent, base, 260);
    if (base[0] == L'\0') {
      return 0;
    }
  }
  if (subkey == nullptr || subkey[0] == L'\0') {
    wcsncpy_s(out, cap, base, _TRUNCATE);
    return 1;
  }
  _snwprintf_s(out, cap, _TRUNCATE, L"%s\\%s", base, subkey);
  return 1;
}

enum VirtualResult {
  kVirtualMiss = 0,
  kVirtualOk = 1,
  kVirtualMoreData = 2,
};

// Size-query (lpData==NULL) returns kVirtualOk and required size (Windows
// RegQueryValueExW/RegGetValueW size-query contract: ERROR_SUCCESS).
static VirtualResult WriteBytes(LPBYTE lpData, LPDWORD lpcbData, const void* src,
                                size_t bytes, DWORD reg_type, LPDWORD lpType) {
  if (lpType) {
    *lpType = reg_type;
  }
  if (lpcbData == nullptr) {
    return kVirtualOk;
  }
  if (lpData == nullptr) {
    *lpcbData = (DWORD)bytes;
    return kVirtualOk;
  }
  if (*lpcbData < bytes) {
    *lpcbData = (DWORD)bytes;
    return kVirtualMoreData;
  }
  memcpy(lpData, src, bytes);
  *lpcbData = (DWORD)bytes;
  return kVirtualOk;
}

static VirtualResult WriteSz(const wchar_t* s, LPDWORD lpType, LPBYTE lpData,
                             LPDWORD lpcbData) {
  size_t bytes = (wcslen(s) + 1) * sizeof(wchar_t);
  return WriteBytes(lpData, lpcbData, s, bytes, REG_SZ, lpType);
}

static VirtualResult WriteDword(DWORD v, LPDWORD lpType, LPBYTE lpData,
                                LPDWORD lpcbData) {
  return WriteBytes(lpData, lpcbData, &v, sizeof(DWORD), REG_DWORD, lpType);
}

// Returns kVirtualMiss when not a known virtual value (Fail Open).
static VirtualResult VirtualValue(const wchar_t* path, const wchar_t* value_name,
                                  LPDWORD lpType, LPBYTE lpData,
                                  LPDWORD lpcbData) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || value_name == nullptr) {
    return kVirtualMiss;
  }

  if (PathEqualsOrUnder(path, kIntlPath)) {
    if (_wcsicmp(value_name, L"LocaleName") == 0 && pfl->has_locale) {
      return WriteSz(pfl->locale_name, lpType, lpData, lpcbData);
    }
    if (_wcsicmp(value_name, L"Locale") == 0 && pfl->has_locale) {
      LCID lcid = EnvBoxProfileLcid();
      wchar_t hex[16];
      _snwprintf_s(hex, _TRUNCATE, L"%08x", (unsigned)lcid);
      return WriteSz(hex, lpType, lpData, lpcbData);
    }
  }

  if (PathEqualsOrUnder(path, kTzPath) && pfl->has_tz) {
    DYNAMIC_TIME_ZONE_INFORMATION info = {};
    if (!EnvBoxLookupTimeZone(pfl->tz_windows, &info)) {
      return kVirtualMiss;
    }
    if (_wcsicmp(value_name, L"TimeZoneKeyName") == 0) {
      return WriteSz(pfl->tz_windows, lpType, lpData, lpcbData);
    }
    if (_wcsicmp(value_name, L"StandardName") == 0) {
      return WriteSz(info.StandardName, lpType, lpData, lpcbData);
    }
    if (_wcsicmp(value_name, L"DaylightName") == 0) {
      return WriteSz(info.DaylightName, lpType, lpData, lpcbData);
    }
    if (_wcsicmp(value_name, L"Bias") == 0) {
      return WriteDword((DWORD)info.Bias, lpType, lpData, lpcbData);
    }
    if (_wcsicmp(value_name, L"StandardBias") == 0) {
      return WriteDword((DWORD)info.StandardBias, lpType, lpData, lpcbData);
    }
    if (_wcsicmp(value_name, L"DaylightBias") == 0) {
      return WriteDword((DWORD)info.DaylightBias, lpType, lpData, lpcbData);
    }
  }
  return kVirtualMiss;
}

static LSTATUS VirtualToStatus(VirtualResult v) {
  if (v == kVirtualOk) {
    return ERROR_SUCCESS;
  }
  if (v == kVirtualMoreData) {
    return ERROR_MORE_DATA;
  }
  return ERROR_SUCCESS;  // unused for miss
}

static LSTATUS WINAPI HookRegOpenKeyExW(HKEY hKey, LPCWSTR lpSubKey,
                                        DWORD ulOptions, REGSAM samDesired,
                                        PHKEY phkResult) {
  LSTATUS st =
      TrueRegOpenKeyExW(hKey, lpSubKey, ulOptions, samDesired, phkResult);
  if (st == ERROR_SUCCESS && phkResult != nullptr && *phkResult != nullptr) {
    wchar_t path[260];
    // Track every open so nested relative opens resolve (Hard #1).
    if (JoinPath(hKey, lpSubKey, path, 260)) {
      TrackKey(*phkResult, path);
    }
  }
  return st;
}

static LSTATUS WINAPI HookRegQueryValueExW(HKEY hKey, LPCWSTR lpValueName,
                                           LPDWORD lpReserved, LPDWORD lpType,
                                           LPBYTE lpData, LPDWORD lpcbData) {
  wchar_t path[260];
  if (LookupPath(hKey, path, 260) && IsWhitelisted(path)) {
    VirtualResult v = VirtualValue(path, lpValueName, lpType, lpData, lpcbData);
    if (v != kVirtualMiss) {
      EnvBoxAuditEventW("RegQueryValueExW", 1,
                        lpValueName ? lpValueName : L"(default)");
      return VirtualToStatus(v);
    }
    EnvBoxAuditEventW("RegQueryValueExW", 0,
                      lpValueName ? lpValueName : L"(default)");
  }
  return TrueRegQueryValueExW(hKey, lpValueName, lpReserved, lpType, lpData,
                              lpcbData);
}

static LSTATUS WINAPI HookRegGetValueW(HKEY hkey, LPCWSTR lpSubKey,
                                       LPCWSTR lpValue, DWORD dwFlags,
                                       LPDWORD pdwType, PVOID pvData,
                                       LPDWORD pcbData) {
  wchar_t path[260];
  if (JoinPath(hkey, lpSubKey, path, 260) && IsWhitelisted(path)) {
    DWORD type = 0;
    LPDWORD type_out = pdwType ? pdwType : &type;
    VirtualResult v =
        VirtualValue(path, lpValue, type_out, (LPBYTE)pvData, pcbData);
    if (v != kVirtualMiss) {
      // Honor RRF_RT_* type mask when the caller set one.
      DWORD want = dwFlags & 0x0000ffff;
      if (want != 0 && want != RRF_RT_ANY) {
        DWORD t = *type_out;
        int match = 0;
        if ((want & RRF_RT_REG_SZ) && t == REG_SZ) match = 1;
        if ((want & RRF_RT_REG_DWORD) && t == REG_DWORD) match = 1;
        if ((want & RRF_RT_REG_MULTI_SZ) && t == REG_MULTI_SZ) match = 1;
        if ((want & RRF_RT_REG_QWORD) && t == REG_QWORD) match = 1;
        if ((want & RRF_RT_REG_BINARY) && t == REG_BINARY) match = 1;
        if ((want & RRF_RT_REG_EXPAND_SZ) && t == REG_EXPAND_SZ) match = 1;
        if (!match) {
          if ((dwFlags & RRF_ZEROONFAILURE) && pvData && pcbData) {
            memset(pvData, 0, *pcbData);
          }
          EnvBoxAuditEventW("RegGetValueW", 1, lpValue ? lpValue : L"(default)");
          return ERROR_UNSUPPORTED_TYPE;
        }
      }
      LSTATUS st = VirtualToStatus(v);
      if (st != ERROR_SUCCESS && (dwFlags & RRF_ZEROONFAILURE) && pvData &&
          pcbData) {
        memset(pvData, 0, *pcbData);
      }
      EnvBoxAuditEventW("RegGetValueW", 1, lpValue ? lpValue : L"(default)");
      return st;
    }
    EnvBoxAuditEventW("RegGetValueW", 0, lpValue ? lpValue : L"(default)");
  }
  return TrueRegGetValueW(hkey, lpSubKey, lpValue, dwFlags, pdwType, pvData,
                          pcbData);
}

static LSTATUS WINAPI HookRegCloseKey(HKEY hKey) {
  UntrackKey(hKey);
  return TrueRegCloseKey(hKey);
}

int EnvBoxInstallRegistryHooks() {
  EnsureLock();
  int ok = 0;
  ok += EnvBoxAttach(&TrueRegOpenKeyExW, HookRegOpenKeyExW);
  ok += EnvBoxAttach(&TrueRegQueryValueExW, HookRegQueryValueExW);
  ok += EnvBoxAttach(&TrueRegGetValueW, HookRegGetValueW);
  ok += EnvBoxAttach(&TrueRegCloseKey, HookRegCloseKey);
  return ok;
}
