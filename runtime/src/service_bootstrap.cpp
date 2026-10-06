#include "service_bootstrap.h"
#include "../../drivers/envbox-policy/service_identity.h"

namespace {
INIT_ONCE g_once = INIT_ONCE_STATIC_INIT;
int g_mode = -1;
BOOL CALLBACK Initialize(PINIT_ONCE, PVOID, PVOID*) {
  wchar_t value[3] = {};
  SetLastError(ERROR_SUCCESS);
  DWORD length = GetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", value, 3);
  DWORD error = GetLastError();
  if (length == 0 && error == ERROR_ENVVAR_NOT_FOUND) g_mode = 0;
  else if (length == 1 && value[0] == L'1') g_mode = 1;
  return TRUE;
}
struct Handle {
  HANDLE value = nullptr;
  ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
bool PrimaryServiceToken(HANDLE token) {
  TOKEN_TYPE type = TokenImpersonation;
  DWORD bytes = 0, session = MAXDWORD;
  if (!GetTokenInformation(token, TokenType, &type, sizeof(type), &bytes) ||
      type != TokenPrimary ||
      !GetTokenInformation(token, TokenSessionId, &session, sizeof(session), &bytes) ||
      session != 0) return false;
  BYTE user_buffer[sizeof(TOKEN_USER) + SECURITY_MAX_SID_SIZE] = {};
  BYTE system_sid[SECURITY_MAX_SID_SIZE] = {};
  DWORD sid_size = sizeof(system_sid);
  if (!CreateWellKnownSid(WinLocalSystemSid, nullptr, system_sid, &sid_size) ||
      !GetTokenInformation(token, TokenUser, user_buffer, sizeof(user_buffer), &bytes) ||
      !EqualSid(reinterpret_cast<TOKEN_USER*>(user_buffer)->User.Sid, system_sid)) return false;
  GetTokenInformation(token, TokenGroups, nullptr, 0, &bytes);
  if (GetLastError() != ERROR_INSUFFICIENT_BUFFER || bytes > 65536 ||
      bytes < sizeof(TOKEN_GROUPS)) return false;
  auto* groups = static_cast<TOKEN_GROUPS*>(HeapAlloc(GetProcessHeap(), 0, bytes));
  if (!groups) return false;
  bool accepted = false;
  DWORD returned = 0;
  if (GetTokenInformation(token, TokenGroups, groups, bytes, &returned)) {
    for (DWORD i = 0; i < groups->GroupCount; ++i) {
      const auto& group = groups->Groups[i];
      if ((group.Attributes & SE_GROUP_ENABLED) &&
          !(group.Attributes & SE_GROUP_USE_FOR_DENY_ONLY) &&
          EqualSid(group.Sid, const_cast<unsigned char*>(eb_service_sid))) {
        accepted = true; break;
      }
    }
  }
  HeapFree(GetProcessHeap(), 0, groups);
  return accepted;
}
}

bool EnvBoxServiceBootstrapConfigurationValid() {
  return InitOnceExecuteOnce(&g_once, Initialize, nullptr, nullptr) && g_mode >= 0;
}
bool EnvBoxServiceBootstrapRequired() {
  return EnvBoxServiceBootstrapConfigurationValid() && g_mode == 1;
}
bool EnvBoxValidateTrustedServicePipe(HANDLE pipe) {
  ULONG before = 0, after = 0;
  if (!GetNamedPipeServerProcessId(pipe, &before) || before == 0) return false;
  Handle process;
  process.value = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, FALSE, before);
  if (!process.value || GetProcessId(process.value) != before) return false;
  Handle token;
  if (!OpenProcessToken(process.value, TOKEN_QUERY, &token.value) ||
      !PrimaryServiceToken(token.value)) return false;
  // The process handle retains the exact object. Reject exit/PID reuse and
  // recheck the connected pipe's actual server after token authentication.
  return WaitForSingleObject(process.value, 0) == WAIT_TIMEOUT &&
      GetNamedPipeServerProcessId(pipe, &after) && before == after;
}

extern "C" const unsigned long EnvBoxTrustedServiceBootstrapCapability = 1;
