// Child process propagation (ticket 06). Process Tree Instance isolation.
// Force CREATE_SUSPENDED -> inject Runtime -> inherit Profile IDs -> Resume
// only when caller_requested_suspended is false.
// Startup Fail Policy: never leave an unvirtualized child running.

#include "hooks.h"

#include <string.h>

#include <string>
#include <utility>
#include <vector>

#include "audit.h"
#include "ipc_bootstrap.h"

static BOOL(WINAPI* TrueCreateProcessW)(
    LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD,
    LPVOID, LPCWSTR, LPSTARTUPINFOW, LPPROCESS_INFORMATION) = CreateProcessW;

static BOOL(WINAPI* TrueCreateProcessA)(
    LPCSTR, LPSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD,
    LPVOID, LPCSTR, LPSTARTUPINFOA, LPPROCESS_INFORMATION) = CreateProcessA;

// CreateProcessW lpEnvironment is ANSI MULTI_SZ unless CREATE_UNICODE_ENVIRONMENT.
// Returns UTF-16 copy of the block (always Unicode for overlay work).
static std::vector<wchar_t> EnvToWide(LPVOID lpEnvironment, DWORD creation_flags) {
  std::vector<wchar_t> out;
  if (lpEnvironment == nullptr) {
    return out;
  }
  if (creation_flags & CREATE_UNICODE_ENVIRONMENT) {
    const wchar_t* p = static_cast<const wchar_t*>(lpEnvironment);
    while (*p) {
      std::wstring entry(p);
      out.insert(out.end(), entry.begin(), entry.end());
      out.push_back(L'\0');
      p += entry.size() + 1;
    }
    out.push_back(L'\0');
    return out;
  }
  // ANSI block.
  const char* p = static_cast<const char*>(lpEnvironment);
  while (*p) {
    std::string entry(p);
    int n = MultiByteToWideChar(CP_ACP, 0, entry.c_str(), -1, nullptr, 0);
    if (n > 0) {
      size_t base = out.size();
      out.resize(base + (size_t)n);
      MultiByteToWideChar(CP_ACP, 0, entry.c_str(), -1, out.data() + base, n);
    }
    p += entry.size() + 1;
  }
  out.push_back(L'\0');
  return out;
}

// Upsert Profile identity + store path into a Unicode MULTI_SZ env block.
static void UpsertProfileKeys(std::vector<wchar_t>* block) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr) {
    return;
  }

  std::vector<std::pair<std::wstring, std::wstring>> vars;
  if (!block->empty()) {
    const wchar_t* p = block->data();
    while (*p) {
      std::wstring entry(p);
      p += entry.size() + 1;
      size_t eq = entry.find(L'=');
      if (eq != std::wstring::npos && eq > 0) {
        vars.emplace_back(entry.substr(0, eq), entry.substr(eq + 1));
      }
    }
  }

  auto upsert = [&](const wchar_t* key, const wchar_t* val) {
    if (val == nullptr) {
      return;
    }
    for (auto& kv : vars) {
      if (_wcsicmp(kv.first.c_str(), key) == 0) {
        kv.second = val;
        return;
      }
    }
    vars.emplace_back(key, val);
  };

  upsert(L"ENVBOX_PROFILE_ID", pfl->profile_id);
  upsert(L"ENVBOX_INSTANCE_ID", pfl->instance_id);
  upsert(L"ENVBOX_INHERIT_CHILDREN", pfl->inherit_children ? L"1" : L"0");
  upsert(L"ENVBOX_AUDIT", pfl->audit ? L"1" : L"0");

  // ENVBOX_* value fallback (V0.3): children can bootstrap without Broker.
  if (pfl->has_locale) {
    upsert(L"ENVBOX_LOCALE_NAME", pfl->locale_name);
  }
  if (pfl->has_ui) {
    upsert(L"ENVBOX_UI_LANGUAGE", pfl->ui_language);
  }
  if (pfl->has_region) {
    upsert(L"ENVBOX_REGION", pfl->region);
  }
  if (pfl->has_tz) {
    upsert(L"ENVBOX_TZ_WINDOWS", pfl->tz_windows);
  }
  if (pfl->tz_iana[0] != L'\0') {
    upsert(L"ENVBOX_TZ_IANA", pfl->tz_iana);
  }
  {
    wchar_t dns_mode[4] = {};
    _snwprintf_s(dns_mode, _TRUNCATE, L"%d", pfl->dns_mode ? 1 : 0);
    upsert(L"ENVBOX_DNS_MODE", dns_mode);
  }
  if (pfl->dns_server_count > 0) {
    std::wstring servers;
    for (int i = 0; i < pfl->dns_server_count; i++) {
      if (i) servers.push_back(L';');
      int n = MultiByteToWideChar(CP_UTF8, 0, pfl->dns_servers[i], -1, nullptr, 0);
      if (n > 0) {
        std::wstring w((size_t)n, L'\0');
        MultiByteToWideChar(CP_UTF8, 0, pfl->dns_servers[i], -1, &w[0], n);
        if (!w.empty() && w.back() == L'\0') w.pop_back();
        servers += w;
      }
    }
    upsert(L"ENVBOX_DNS_SERVERS", servers.c_str());
  }
  if (pfl->registry_path_count > 0) {
    std::wstring paths;
    for (int i = 0; i < pfl->registry_path_count; i++) {
      if (i) paths.push_back(L';');
      paths += pfl->registry_paths[i];
    }
    upsert(L"ENVBOX_REGISTRY_PATHS", paths.c_str());
  }

  wchar_t root[MAX_PATH] = {};
  if (GetEnvironmentVariableW(L"ENVBOX_CONFIG_ROOT", root, MAX_PATH) > 0) {
    upsert(L"ENVBOX_CONFIG_ROOT", root);
  }
  wchar_t rt[MAX_PATH] = {};
  if (GetEnvironmentVariableW(L"ENVBOX_RUNTIME_DLL", rt, MAX_PATH) > 0) {
    upsert(L"ENVBOX_RUNTIME_DLL", rt);
  }
  wchar_t pipe[MAX_PATH] = {};
  if (GetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", pipe, MAX_PATH) > 0) {
    upsert(L"ENVBOX_IPC_PIPE", pipe);
  }

  block->clear();
  for (auto& kv : vars) {
    block->insert(block->end(), kv.first.begin(), kv.first.end());
    block->push_back(L'=');
    block->insert(block->end(), kv.second.begin(), kv.second.end());
    block->push_back(L'\0');
  }
  block->push_back(L'\0');
}

// Copy the current process Unicode environment (GetEnvironmentStringsW) as MULTI_SZ.
static std::vector<wchar_t> CurrentProcessEnvBlock() {
  std::vector<wchar_t> out;
  LPWCH blk = GetEnvironmentStringsW();
  if (blk == nullptr) {
    return out;
  }
  const wchar_t* p = blk;
  while (*p) {
    std::wstring entry(p);
    out.insert(out.end(), entry.begin(), entry.end());
    out.push_back(L'\0');
    p += entry.size() + 1;
  }
  out.push_back(L'\0');
  FreeEnvironmentStringsW(blk);
  return out;
}

// Returns 0 on failure and sets last_error. On success the child is running
// (or still suspended if the caller asked for that).
// Use DetourCreateProcessWithDllExW (same as Root launch) with TrueCreateProcessW
// to avoid re-entering this hook.
static BOOL SpawnInjected(
    LPCWSTR lpApplicationName, LPWSTR lpCommandLine,
    LPSECURITY_ATTRIBUTES lpProcessAttributes,
    LPSECURITY_ATTRIBUTES lpThreadAttributes, BOOL bInheritHandles,
    DWORD dwCreationFlags, LPVOID lpEnvironment, LPCWSTR lpCurrentDirectory,
    LPSTARTUPINFOW lpStartupInfo, LPPROCESS_INFORMATION lpProcessInformation) {
  const char* dll = EnvBoxRuntimeDllPathA();
  if (dll == nullptr) {
    // Startup Fail Policy: never create an unvirtualized child.
    SetLastError(ERROR_MOD_NOT_FOUND);
    EnvBoxAuditEvent("CreateProcessW", 0, "no-runtime-dll");
    return FALSE;
  }

  const DWORD caller_requested_suspended = (dwCreationFlags & CREATE_SUSPENDED);
  DWORD flags = dwCreationFlags | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;

  std::vector<wchar_t> env = EnvToWide(lpEnvironment, dwCreationFlags);
  if (env.empty()) {
    // lpEnvironment == nullptr means "inherit"; never replace with ENVBOX-only.
    env = CurrentProcessEnvBlock();
  }
  UpsertProfileKeys(&env);
  LPVOID env_ptr = env.empty() ? lpEnvironment : static_cast<LPVOID>(env.data());

  if (!DetourCreateProcessWithDllExW(
          lpApplicationName, lpCommandLine, lpProcessAttributes,
          lpThreadAttributes, bInheritHandles, flags, env_ptr,
          lpCurrentDirectory, lpStartupInfo, lpProcessInformation, dll,
          reinterpret_cast<PDETOUR_CREATE_PROCESS_ROUTINEW>(TrueCreateProcessW))) {
    DWORD err = GetLastError();
    if (lpProcessInformation != nullptr) {
      if (lpProcessInformation->hThread != nullptr) {
        CloseHandle(lpProcessInformation->hThread);
        lpProcessInformation->hThread = nullptr;
      }
      if (lpProcessInformation->hProcess != nullptr) {
        CloseHandle(lpProcessInformation->hProcess);
        lpProcessInformation->hProcess = nullptr;
      }
      lpProcessInformation->dwProcessId = 0;
    }
    SetLastError(err);
    // Ticket 30: surface elevation/integrity vs generic inject failure (audit only).
    if (err == ERROR_ELEVATION_REQUIRED || err == ERROR_ACCESS_DENIED ||
        err == ERROR_PRIVILEGE_NOT_HELD) {
      EnvBoxAuditEvent("CreateProcessW", 0, "inject-failed-elevation");
    } else {
      EnvBoxAuditEvent("CreateProcessW", 0, "inject-failed");
    }
    return FALSE;
  }

  if (!caller_requested_suspended) {
    if (ResumeThread(lpProcessInformation->hThread) == (DWORD)-1) {
      DWORD err = GetLastError();
      TerminateProcess(lpProcessInformation->hProcess, 1);
      CloseHandle(lpProcessInformation->hThread);
      CloseHandle(lpProcessInformation->hProcess);
      lpProcessInformation->dwProcessId = 0;
      lpProcessInformation->hProcess = nullptr;
      lpProcessInformation->hThread = nullptr;
      SetLastError(err);
      EnvBoxAuditEvent("CreateProcessW", 0, "inject-resume-failed");
      return FALSE;
    }
  }
  // Never log command bodies or environment blocks (ticket 21).
  EnvBoxAuditEvent("CreateProcessW", 1,
                   caller_requested_suspended ? "inject-suspended" : "inject-resumed");
  // V0.3: notify Broker so the child joins this EnvironmentSession.
  if (lpProcessInformation != nullptr) {
    EnvBoxIpcNotifyProcessCreated(lpProcessInformation->dwProcessId, nullptr);
  }
  return TRUE;
}

static BOOL WINAPI HookCreateProcessW(
    LPCWSTR lpApplicationName, LPWSTR lpCommandLine,
    LPSECURITY_ATTRIBUTES lpProcessAttributes,
    LPSECURITY_ATTRIBUTES lpThreadAttributes, BOOL bInheritHandles,
    DWORD dwCreationFlags, LPVOID lpEnvironment, LPCWSTR lpCurrentDirectory,
    LPSTARTUPINFOW lpStartupInfo, LPPROCESS_INFORMATION lpProcessInformation) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr) {
    // No profile: cannot virtualize; reject (Startup Fail Policy).
    SetLastError(ERROR_INVALID_DATA);
    EnvBoxAuditEvent("CreateProcessW", 0, "no-profile");
    return FALSE;
  }
  if (!pfl->inherit_children) {
    // Application opted out of child propagation: plain create (root-only view).
    EnvBoxAuditEvent("CreateProcessW", 0, "inherit-off-plain");
    return TrueCreateProcessW(lpApplicationName, lpCommandLine,
                              lpProcessAttributes, lpThreadAttributes,
                              bInheritHandles, dwCreationFlags, lpEnvironment,
                              lpCurrentDirectory, lpStartupInfo,
                              lpProcessInformation);
  }
  return SpawnInjected(lpApplicationName, lpCommandLine, lpProcessAttributes,
                       lpThreadAttributes, bInheritHandles, dwCreationFlags,
                       lpEnvironment, lpCurrentDirectory, lpStartupInfo,
                       lpProcessInformation);
}

static BOOL WINAPI HookCreateProcessA(
    LPCSTR lpApplicationName, LPSTR lpCommandLine,
    LPSECURITY_ATTRIBUTES lpProcessAttributes,
    LPSECURITY_ATTRIBUTES lpThreadAttributes, BOOL bInheritHandles,
    DWORD dwCreationFlags, LPVOID lpEnvironment, LPCSTR lpCurrentDirectory,
    LPSTARTUPINFOA lpStartupInfo, LPPROCESS_INFORMATION lpProcessInformation) {
  wchar_t app[32768] = {};
  wchar_t cmd[32768] = {};
  wchar_t cwd[32768] = {};
  if (lpApplicationName) {
    MultiByteToWideChar(CP_ACP, 0, lpApplicationName, -1, app, 32768);
  }
  if (lpCommandLine) {
    MultiByteToWideChar(CP_ACP, 0, lpCommandLine, -1, cmd, 32768);
  }
  if (lpCurrentDirectory) {
    MultiByteToWideChar(CP_ACP, 0, lpCurrentDirectory, -1, cwd, 32768);
  }

  STARTUPINFOW siw = {};
  if (lpStartupInfo) {
    siw.cb = sizeof(siw);
    siw.lpDesktop = nullptr;
    siw.lpTitle = nullptr;
    siw.dwFlags = lpStartupInfo->dwFlags;
    siw.wShowWindow = lpStartupInfo->wShowWindow;
    siw.dwX = lpStartupInfo->dwX;
    siw.dwY = lpStartupInfo->dwY;
    siw.dwXSize = lpStartupInfo->dwXSize;
    siw.dwYSize = lpStartupInfo->dwYSize;
    siw.dwXCountChars = lpStartupInfo->dwXCountChars;
    siw.dwYCountChars = lpStartupInfo->dwYCountChars;
    siw.dwFillAttribute = lpStartupInfo->dwFillAttribute;
    siw.hStdInput = lpStartupInfo->hStdInput;
    siw.hStdOutput = lpStartupInfo->hStdOutput;
    siw.hStdError = lpStartupInfo->hStdError;
  } else {
    siw.cb = sizeof(siw);
  }

  return HookCreateProcessW(
      app[0] ? app : nullptr, cmd[0] ? cmd : nullptr, lpProcessAttributes,
      lpThreadAttributes, bInheritHandles, dwCreationFlags, lpEnvironment,
      cwd[0] ? cwd : nullptr, &siw, lpProcessInformation);
}

int EnvBoxInstallProcessHooks() {
  int ok = 0;
  ok += EnvBoxAttach(&TrueCreateProcessW, HookCreateProcessW);
  ok += EnvBoxAttach(&TrueCreateProcessA, HookCreateProcessA);
  return ok;
}
