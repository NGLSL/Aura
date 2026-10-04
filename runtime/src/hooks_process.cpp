// Child process propagation (ticket 06). Process Tree Instance isolation.
// Force CREATE_SUSPENDED -> inject Runtime -> inherit Profile IDs -> Resume
// only when caller_requested_suspended is false.
// Startup Fail Policy: never leave an unvirtualized child running.

#include "hooks.h"

#include <algorithm>
#include <string.h>
#include <wctype.h>

#include <string>
#include <utility>
#include <vector>

#include "audit.h"
#include "ipc_bootstrap.h"

static bool ControlledStartup() {
  wchar_t value[8] = {};
  return GetEnvironmentVariableW(L"ENVBOX_STARTUP_GATE", value, 8) == 1 && value[0] == L'1';
}

// --- Browser / Network Guard (ticket 54): Child Guard decision table ----------
// Explicit engine image names only (never bare substring "chrome").
enum BrowserEngineKind {
  kEngineUnknown = 0,
  kEngineChromium = 1,
  kEngineEdge = 2,
  kEngineWebView2 = 3,
  kEngineElectron = 4,
};

static BrowserEngineKind ClassifyBrowserEngine(const wchar_t* path_or_cmd) {
  if (path_or_cmd == nullptr || path_or_cmd[0] == L'\0') {
    return kEngineUnknown;
  }
  // Take the file name component.
  const wchar_t* name = path_or_cmd;
  for (const wchar_t* p = path_or_cmd; *p; ++p) {
    if (*p == L'\\' || *p == L'/') {
      name = p + 1;
    }
  }
  // Strip surrounding quotes if present.
  std::wstring n(name);
  if (!n.empty() && n.front() == L'"') {
    n.erase(0, 1);
  }
  // Cut at first space (command line token).
  size_t sp = n.find(L' ');
  if (sp != std::wstring::npos) {
    n = n.substr(0, sp);
  }
  // Lowercase for compare.
  for (auto& c : n) {
    if (c >= L'A' && c <= L'Z') c = (wchar_t)(c - L'A' + L'a');
  }
  if (n == L"msedgewebview2.exe" || n.rfind(L"msedgewebview2", 0) == 0) {
    return kEngineWebView2;
  }
  if (n == L"electron.exe" || (n.size() >= 12 &&
                              n.compare(n.size() - 12, 12, L"electron.exe") == 0)) {
    return kEngineElectron;
  }
  if (n == L"msedge.exe") {
    return kEngineEdge;
  }
  if (n == L"chrome.exe" || n == L"chromium.exe" || n == L"chrome_proxy.exe" ||
      n == L"googlechromeproxy.exe") {
    return kEngineChromium;
  }
  return kEngineUnknown;
}

// 0=host, 1=public_interface_only, 2=proxy_only, 3=strict
static int WebrtcPolicyCode(const RuntimeProfile* pfl) {
  if (pfl == nullptr || pfl->webrtc_policy[0] == L'\0') {
    return 0;
  }
  if (_wcsicmp(pfl->webrtc_policy, L"public_interface_only") == 0) return 1;
  if (_wcsicmp(pfl->webrtc_policy, L"proxy_only") == 0) return 2;
  if (_wcsicmp(pfl->webrtc_policy, L"strict") == 0) return 3;
  return 0;
}

static const wchar_t* ChromiumIpHandlingValue(int policy_code) {
  switch (policy_code) {
    case 1:
      return L"default_public_interface_only";
    case 2:
    case 3:
      return L"disable_non_proxied_udp";
    default:
      return nullptr;
  }
}

static const wchar_t* kChromiumSwitch = L"--force-webrtc-ip-handling-policy";

// Rewrite/append Chromium WebRTC switch on a command line. Never duplicates.
// Returns 1 if modified, 0 if unchanged. Fail Open on buffer issues.
static int EnsureChromiumSwitchW(std::wstring* cmd, int policy_code) {
  const wchar_t* value = ChromiumIpHandlingValue(policy_code);
  if (value == nullptr) {
    return 0;
  }
  std::wstring want = std::wstring(kChromiumSwitch) + L"=" + value;
  std::wstring lower = *cmd;
  for (auto& c : lower) {
    if (c >= L'A' && c <= L'Z') c = (wchar_t)(c - L'A' + L'a');
  }
  size_t pos = lower.find(kChromiumSwitch);
  if (pos != std::wstring::npos) {
    // Rewrite existing token: find end of this argv token.
    size_t end = pos + wcslen(kChromiumSwitch);
    // Skip =value or " value"
    if (end < cmd->size() && (*cmd)[end] == L'=') {
      end++;
      while (end < cmd->size() && !iswspace((*cmd)[end])) end++;
    } else {
      while (end < cmd->size() && iswspace((*cmd)[end])) end++;
      while (end < cmd->size() && !iswspace((*cmd)[end])) end++;
    }
    cmd->replace(pos, end - pos, want);
    return 1;
  }
  if (!cmd->empty() && !iswspace(cmd->back())) {
    cmd->push_back(L' ');
  }
  cmd->append(want);
  return 1;
}

// Upsert WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS with the policy switch.
static int EnsureWebView2ArgsW(std::vector<std::pair<std::wstring, std::wstring>>* vars,
                               int policy_code) {
  const wchar_t* value = ChromiumIpHandlingValue(policy_code);
  if (value == nullptr) {
    return 0;
  }
  std::wstring want = std::wstring(kChromiumSwitch) + L"=" + value;
  const wchar_t* key = L"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
  for (auto& kv : *vars) {
    if (_wcsicmp(kv.first.c_str(), key) == 0) {
      std::wstring lower = kv.second;
      for (auto& c : lower) {
        if (c >= L'A' && c <= L'Z') c = (wchar_t)(c - L'A' + L'a');
      }
      size_t pos = lower.find(kChromiumSwitch);
      if (pos != std::wstring::npos) {
        size_t end = pos + wcslen(kChromiumSwitch);
        if (end < kv.second.size() && kv.second[end] == L'=') {
          end++;
          while (end < kv.second.size() && !iswspace(kv.second[end])) end++;
        } else {
          while (end < kv.second.size() && iswspace(kv.second[end])) end++;
          while (end < kv.second.size() && !iswspace(kv.second[end])) end++;
        }
        kv.second.replace(pos, end - pos, want);
      } else {
        if (!kv.second.empty() && !iswspace(kv.second.back())) {
          kv.second.push_back(L' ');
        }
        kv.second.append(want);
      }
      return 1;
    }
  }
  vars->emplace_back(key, want);
  return 1;
}

// Apply Browser Policy to child command line + env before create.
// Unknown engines leave the command line alone. Host policy: no changes.
static void ApplyBrowserChildPolicy(const wchar_t* image_or_cmd, std::wstring* cmd,
                                    std::vector<std::pair<std::wstring, std::wstring>>* vars) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr) {
    return;
  }
  int policy_code = WebrtcPolicyCode(pfl);
  if (policy_code == 0) {
    return;  // Host: never overwrite user args.
  }
  // Classify from application name or first token of command line.
  BrowserEngineKind engine = ClassifyBrowserEngine(image_or_cmd);
  if (engine == kEngineUnknown && cmd != nullptr) {
    engine = ClassifyBrowserEngine(cmd->c_str());
  }
  if (engine == kEngineUnknown) {
    if (pfl->audit) {
      EnvBoxAuditEvent("CreateProcessW", 1, "BrowserPolicyIgnoredUnknown");
    }
    return;
  }
  if (engine == kEngineWebView2) {
    if (EnsureWebView2ArgsW(vars, policy_code) && pfl->audit) {
      EnvBoxAuditEvent("CreateProcessW", 1, "BrowserPolicyApplied");
    }
    return;
  }
  // Chromium / Edge / Electron: command-line switch.
  if (cmd != nullptr && EnsureChromiumSwitchW(cmd, policy_code) && pfl->audit) {
    EnvBoxAuditEvent("CreateProcessW", 1, "BrowserPolicyApplied");
  }
}

static BOOL(WINAPI* TrueCreateProcessW)(
    LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD,
    LPVOID, LPCWSTR, LPSTARTUPINFOW, LPPROCESS_INFORMATION) = CreateProcessW;

static BOOL(WINAPI* TrueCreateProcessA)(
    LPCSTR, LPSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD,
    LPVOID, LPCSTR, LPSTARTUPINFOA, LPPROCESS_INFORMATION) = CreateProcessA;

static BOOL(WINAPI* TrueCreateProcessAsUserW)(
    HANDLE, LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES,
    BOOL, DWORD, LPVOID, LPCWSTR, LPSTARTUPINFOW,
    LPPROCESS_INFORMATION) = CreateProcessAsUserW;

// Detours accepts a CreateProcessW-shaped callback. Keep the AsUser token on
// this thread while Detours synchronously invokes the callback so the target
// retains Chrome's restricted token and sandbox startup attributes.
static thread_local HANDLE g_asuser_token = nullptr;

static BOOL WINAPI CreateProcessAsUserAdapter(
    LPCWSTR app, LPWSTR cmd, LPSECURITY_ATTRIBUTES process_attributes,
    LPSECURITY_ATTRIBUTES thread_attributes, BOOL inherit_handles, DWORD flags,
    LPVOID environment, LPCWSTR current_directory, LPSTARTUPINFOW startup,
    LPPROCESS_INFORMATION process_info) {
  return TrueCreateProcessAsUserW(
      g_asuser_token, app, cmd, process_attributes, thread_attributes,
      inherit_handles, flags, environment, current_directory, startup,
      process_info);
}

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
static bool UpsertProfileKeys(std::vector<wchar_t>* block) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr) {
    return false;
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

  // With the complete IPC Profile, its locale takes precedence over inherited
  // and caller-supplied child POSIX locale values. Profile-supplied values
  // are applied after removal.
  // ENVBOX_* fallback lacks those overrides, so preserve the already-merged
  // inherited block instead of discarding an explicit Profile LANG.
  if (EnvBoxProfileEnvironmentComplete()) {
    vars.erase(std::remove_if(vars.begin(), vars.end(), [](const auto& kv) {
                 const wchar_t* key = kv.first.c_str();
                 return _wcsicmp(key, L"LANG") == 0 ||
                        _wcsicmp(key, L"LANGUAGE") == 0 ||
                        _wcsnicmp(key, L"LC_", 3) == 0;
               }),
               vars.end());
  }

  // EnvironmentProfile overrides. Apply these before internal identity so a
  // user value can never replace ENVBOX_* bookkeeping.
  for (int i = 0; i < pfl->environment_count && i < ENVBOX_ENV_MAX; i++) {
    const wchar_t* entry = pfl->environment[i];
    const wchar_t* eq = wcschr(entry, L'=');
    if (eq == nullptr || eq == entry) {
      continue;
    }
    std::wstring key(entry, (size_t)(eq - entry));
    upsert(key.c_str(), eq + 1);
  }

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
  // Rebuild the full ordered DNS snapshot. Caller-supplied stale indexed
  // fields must not survive alongside the immutable parent's configuration.
  vars.erase(std::remove_if(vars.begin(), vars.end(), [](const auto& kv) {
    return _wcsnicmp(kv.first.c_str(), L"ENVBOX_DNS_", 11) == 0;
  }), vars.end());
  auto dns_setter = [](void* context, const char* key, const char* value) -> int {
    auto* values = static_cast<std::vector<std::pair<std::wstring, std::wstring>>*>(context);
    std::wstring name = L"ENVBOX_";
    for (const char* p = key; *p; ++p) name += static_cast<wchar_t>(*p >= 'a' && *p <= 'z' ? *p - 'a' + 'A' : *p);
    int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value, -1, nullptr, 0);
    if (size <= 0) return 0;
    std::wstring text(static_cast<size_t>(size), L'\0');
    if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value, -1, text.data(), size)) return 0;
    text.pop_back(); values->emplace_back(name, text); return 1;
  };
  if (!EnvBoxEmitDnsConfiguration(pfl, dns_setter, &vars)) return false;
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
  // Carry only the owning parent's escrow identity. Remove caller spoofing,
  // including duplicates, before inserting the verified parent value.
  vars.erase(std::remove_if(vars.begin(), vars.end(), [](const auto& kv) {
      return _wcsicmp(kv.first.c_str(), L"ENVBOX_RECOVERY_JOB_NAME") == 0;
    }), vars.end());
  wchar_t recovery_job[256] = {};
  SetLastError(ERROR_SUCCESS);
  DWORD recovery_length = GetEnvironmentVariableW(L"ENVBOX_RECOVERY_JOB_NAME", recovery_job, 256);
  if (recovery_length >= 256 || (recovery_length == 0 && GetLastError() != ERROR_ENVVAR_NOT_FOUND)) return false;
  if (recovery_length > 0) upsert(L"ENVBOX_RECOVERY_JOB_NAME", recovery_job);
  // A caller-supplied environment cannot silently remove a controlled
  // parent's entry gate from its child.
  wchar_t gate[8] = {};
  if (GetEnvironmentVariableW(L"ENVBOX_STARTUP_GATE", gate, 8) == 1 && gate[0] == L'1') {
    upsert(L"ENVBOX_STARTUP_GATE", L"1");
  }

  // Browser / Network Guard: always carry the policy token + WebView2 args.
  {
    const wchar_t* tok =
        (pfl->webrtc_policy[0] != L'\0') ? pfl->webrtc_policy : L"host";
    upsert(L"ENVBOX_WEBRTC_POLICY", tok);
    int policy_code = WebrtcPolicyCode(pfl);
    if (policy_code != 0) {
      EnsureWebView2ArgsW(&vars, policy_code);
    }
  }

  block->clear();
  for (auto& kv : vars) {
    block->insert(block->end(), kv.first.begin(), kv.first.end());
    block->push_back(L'=');
    block->insert(block->end(), kv.second.begin(), kv.second.end());
    block->push_back(L'\0');
  }
  block->push_back(L'\0');
  return true;
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

// Only report exit after OS evidence. A failed kill/wait leaves the suspended
// member visible to its owning Job/Registry and reports cleanup uncertainty.
static DWORD CleanupFailedChild(LPPROCESS_INFORMATION child, const char* api) {
  DWORD failure = ERROR_SUCCESS;
  if (!TerminateProcess(child->hProcess, 1)) failure = GetLastError();
  DWORD wait = WaitForSingleObject(child->hProcess, 3000);
  if (wait == WAIT_OBJECT_0) {
    EnvBoxIpcNotifyProcessExitedPid(child->dwProcessId, 1);
    failure = ERROR_SUCCESS;
  } else {
    if (failure == ERROR_SUCCESS) failure = wait == WAIT_TIMEOUT ? ERROR_TIMEOUT : GetLastError();
    char detail[96];
    _snprintf_s(detail, sizeof(detail), _TRUNCATE, "child-cleanup-unconfirmed pid=%lu error=%lu", child->dwProcessId, failure);
    EnvBoxIpcNotifyHookError(api, failure, detail);
    EnvBoxAuditEvent(api, 0, detail);
  }
  CloseHandle(child->hThread);
  CloseHandle(child->hProcess);
  ZeroMemory(child, sizeof(*child));
  return failure;
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
    LPSTARTUPINFOW lpStartupInfo, LPPROCESS_INFORMATION lpProcessInformation,
    PDETOUR_CREATE_PROCESS_ROUTINEW create_process,
    const char* audit_api) {
  const char* dll = EnvBoxRuntimeDllPathA();
  if (dll == nullptr) {
    // Startup Fail Policy: never create an unvirtualized child.
    EnvBoxAuditEvent(audit_api, 0, "no-runtime-dll");
    SetLastError(ERROR_MOD_NOT_FOUND);
    return FALSE;
  }

  const DWORD caller_requested_suspended = (dwCreationFlags & CREATE_SUSPENDED);
  DWORD flags = dwCreationFlags | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;

  std::vector<wchar_t> env = EnvToWide(lpEnvironment, dwCreationFlags);
  if (env.empty()) {
    // lpEnvironment == nullptr means "inherit"; never replace with ENVBOX-only.
    env = CurrentProcessEnvBlock();
  }
  if (!UpsertProfileKeys(&env)) { EnvBoxAuditEvent(audit_api, 0, "child-dns-snapshot-invalid"); SetLastError(ERROR_INVALID_DATA); return FALSE; }
  LPVOID env_ptr = env.empty() ? lpEnvironment : static_cast<LPVOID>(env.data());

  // Browser Child Guard (ticket 54): apply WebRTC policy to browser engines.
  std::wstring cmd_storage;
  LPWSTR cmd_ptr = lpCommandLine;
  if (lpCommandLine != nullptr) {
    cmd_storage.assign(lpCommandLine);
    // Rebuild env vars overlay so WebView2 args land before create.
    std::vector<std::pair<std::wstring, std::wstring>> dummy;  // env already upserted
    ApplyBrowserChildPolicy(lpApplicationName, &cmd_storage, &dummy);
    // WebView2 env is applied inside UpsertProfileKeys; here we only need the
    // command-line rewrite for Chromium/Edge/Electron. Re-parse if changed.
    if (cmd_storage != lpCommandLine) {
      cmd_ptr = cmd_storage.data();
    }
  } else if (lpApplicationName != nullptr) {
    std::wstring only_app(lpApplicationName);
    std::vector<std::pair<std::wstring, std::wstring>> dummy;
    ApplyBrowserChildPolicy(lpApplicationName, &only_app, &dummy);
    if (only_app != lpApplicationName) {
      cmd_storage = only_app;
      cmd_ptr = cmd_storage.data();
    }
  }

  if (!DetourCreateProcessWithDllExW(
          lpApplicationName, cmd_ptr, lpProcessAttributes,
          lpThreadAttributes, bInheritHandles, flags, env_ptr,
          lpCurrentDirectory, lpStartupInfo, lpProcessInformation, dll,
          create_process)) {
    DWORD err = GetLastError();
    if (lpProcessInformation != nullptr) {
      // Detours owns failed creation: its injection-failure path terminates
      // the new process and closes both handles before returning FALSE.
      // Never close these stale values (a concurrent open can reuse them).
      ZeroMemory(lpProcessInformation, sizeof(*lpProcessInformation));
    }
    // Ticket 30: surface elevation/integrity vs generic inject failure (audit only).
    if (err == ERROR_ELEVATION_REQUIRED || err == ERROR_ACCESS_DENIED ||
        err == ERROR_PRIVILEGE_NOT_HELD) {
      EnvBoxAuditEvent(audit_api, 0, "inject-failed-elevation");
    } else {
      EnvBoxAuditEvent(audit_api, 0, "inject-failed");
    }
    SetLastError(err);
    return FALSE;
  }

  // Bind the child before its loader runs. The injected DLL asks the Broker
  // for its Profile during DllMain, so notifying after ResumeThread races and
  // can return an empty Profile.
  if (lpProcessInformation != nullptr) {
    wchar_t gate[8] = {};
    bool controlled = GetEnvironmentVariableW(L"ENVBOX_STARTUP_GATE", gate, 8) == 1 && gate[0] == L'1';
    // A complete broker Profile requires a sealed child DLL expectation even
    // without an entry gate: its loader must receive a real identity ACK.
    bool requires_binding = controlled || EnvBoxProfileEnvironmentComplete();
    if (requires_binding && !EnvBoxIpcRegisterChild(lpProcessInformation->hProcess, lpProcessInformation->dwProcessId)) {
      DWORD error = GetLastError();
      DWORD cleanup = CleanupFailedChild(lpProcessInformation, audit_api);
      EnvBoxAuditEvent(audit_api, 0, "controlled-child-binding-failed");
      SetLastError(cleanup != ERROR_SUCCESS ? cleanup : error);
      return FALSE;
    }
    if (!requires_binding) EnvBoxIpcNotifyProcessCreated(lpProcessInformation->dwProcessId, nullptr);
  }

  if (!caller_requested_suspended) {
    if (ResumeThread(lpProcessInformation->hThread) == (DWORD)-1) {
      DWORD err = GetLastError();
      DWORD cleanup = CleanupFailedChild(lpProcessInformation, audit_api);
      EnvBoxAuditEvent(audit_api, 0, "inject-resume-failed");
      SetLastError(cleanup != ERROR_SUCCESS ? cleanup : err);
      return FALSE;
    }
  }
  // Never log command bodies or environment blocks (ticket 21). The child PID
  // lets Audit Mode correlate a successful Detours patch with a live process.
  char summary[80] = {};
  _snprintf_s(summary, sizeof(summary), _TRUNCATE, "%s child-pid=%lu",
              caller_requested_suspended ? "inject-suspended" : "inject-resumed",
              (unsigned long)lpProcessInformation->dwProcessId);
  EnvBoxAuditEvent(audit_api, 1, summary);
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
    EnvBoxAuditEvent("CreateProcessW", 0, "no-profile");
    SetLastError(ERROR_INVALID_DATA);
    return FALSE;
  }
  if (!pfl->inherit_children) {
    if (ControlledStartup()) { EnvBoxAuditEvent("CreateProcessW", 0, "controlled-child-inherit-off-unsupported"); SetLastError(ERROR_NOT_SUPPORTED); return FALSE; }
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
                       lpProcessInformation, TrueCreateProcessW,
                       "CreateProcessW");
}

static bool HasCommandSwitch(const wchar_t* command, const wchar_t* wanted) {
  if (command == nullptr) return false;
  size_t length = wcslen(wanted);
  for (const wchar_t* match = command;
       (match = wcsstr(match, wanted)) != nullptr; match += length) {
    if ((match == command || iswspace(match[-1])) &&
        (match[length] == L'\0' || iswspace(match[length]))) {
      return true;
    }
  }
  return false;
}

static BOOL WINAPI HookCreateProcessAsUserW(
    HANDLE token, LPCWSTR app, LPWSTR cmd,
    LPSECURITY_ATTRIBUTES process_attributes,
    LPSECURITY_ATTRIBUTES thread_attributes, BOOL inherit_handles, DWORD flags,
    LPVOID environment, LPCWSTR current_directory, LPSTARTUPINFOW startup,
    LPPROCESS_INFORMATION process_info) {
  const RuntimeProfile* profile = EnvBoxProfile();
  if (profile == nullptr) {
    SetLastError(ERROR_INVALID_DATA);
    EnvBoxAuditEvent("CreateProcessAsUserW", 0, "no-profile");
    return FALSE;
  }
  if (!profile->inherit_children) {
    if (ControlledStartup()) { EnvBoxAuditEvent("CreateProcessAsUserW", 0, "controlled-child-inherit-off-unsupported"); SetLastError(ERROR_NOT_SUPPORTED); return FALSE; }
    EnvBoxAuditEvent("CreateProcessAsUserW", 0, "inherit-off-plain");
    return TrueCreateProcessAsUserW(
        token, app, cmd, process_attributes, thread_attributes, inherit_handles,
        flags, environment, current_directory, startup, process_info);
  }

  // Chromium's sandboxed renderer rejects third-party Runtime DLLs. Detours
  // can patch its suspended image yet the module is absent after startup;
  // repeatedly attempting this path causes renderer restart churn. Keep the
  // browser's own sandbox creation intact and report this partial coverage.
  BrowserEngineKind engine = ClassifyBrowserEngine(app);
  if ((engine == kEngineChromium || engine == kEngineEdge) &&
      HasCommandSwitch(cmd, L"--type=renderer")) {
    if (ControlledStartup()) { EnvBoxAuditEvent("CreateProcessAsUserW", 0, "controlled-chromium-renderer-unsupported"); SetLastError(ERROR_NOT_SUPPORTED); return FALSE; }
    BOOL created = TrueCreateProcessAsUserW(
        token, app, cmd, process_attributes, thread_attributes, inherit_handles,
        flags, environment, current_directory, startup, process_info);
    DWORD error = GetLastError();
    EnvBoxAuditEvent("CreateProcessAsUserW", 0,
                     "chromium-sandboxed-renderer-unsupported");
    SetLastError(error);
    return created;
  }

  struct TokenScope {
    HANDLE previous;
    explicit TokenScope(HANDLE token) : previous(g_asuser_token) {
      g_asuser_token = token;
    }
    ~TokenScope() { g_asuser_token = previous; }
  } token_scope(token);
  return SpawnInjected(
      app, cmd, process_attributes, thread_attributes, inherit_handles, flags,
      environment, current_directory, startup, process_info,
      CreateProcessAsUserAdapter, "CreateProcessAsUserW");
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
  ok += EnvBoxAttach(&TrueCreateProcessAsUserW, HookCreateProcessAsUserW);
  return ok;
}
