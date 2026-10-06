#include "../../runtime/src/service_bootstrap.h"
#include <sddl.h>
#include <stdio.h>
#include <string>

static int Check(bool condition, const char* label) {
  printf("%s %s\n", condition ? "PASS" : "FAIL", label);
  return condition ? 0 : 1;
}
static void Profile() {
  SetEnvironmentVariableW(L"ENVBOX_PROFILE_ID", L"c6ee9aa5-b0a2-4a3c-8a54-f59971075a22");
  SetEnvironmentVariableW(L"ENVBOX_INSTANCE_ID", L"c6ee9aa5-b0a2-4a3c-8a54-f59971075a23");
  SetEnvironmentVariableW(L"ENVBOX_LOCALE_NAME", L"en-US");
  SetEnvironmentVariableW(L"ENVBOX_UI_LANGUAGE", L"en-US");
  SetEnvironmentVariableW(L"ENVBOX_REGION", L"US");
  SetEnvironmentVariableW(L"ENVBOX_TZ_WINDOWS", L"UTC");
  SetEnvironmentVariableW(L"ENVBOX_DNS_MODE", L"host");
}
int wmain(int argc, wchar_t** argv) {
  if (argc < 2) return 2;
  SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", nullptr);
  std::wstring mode = argv[1];
  if (mode.compare(0, 5, L"flag-") == 0) {
    bool valid = true, required = false;
    if (mode == L"flag-one" || mode == L"flag-one-clear") {
      SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L"1"); required = true;
    } else if (mode == L"flag-zero") {
      SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L"0"); valid = false;
    } else if (mode == L"flag-empty") {
      SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L""); valid = false;
    } else if (mode == L"flag-long") {
      SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L"111111111111111111"); valid = false;
    } else if (mode == L"flag-invalid-one") {
      SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L"bad"); valid = false;
    }
    int failures = Check(EnvBoxServiceBootstrapConfigurationValid() == valid, "initial_configuration");
    failures += Check(EnvBoxServiceBootstrapRequired() == required, "initial_mode");
    SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP",
        mode == L"flag-one-clear" ? nullptr : L"1");
    failures += Check(EnvBoxServiceBootstrapConfigurationValid() == valid, "latched_configuration");
    failures += Check(EnvBoxServiceBootstrapRequired() == required, "latched_mode");
    return failures ? 1 : 0;
  }
  wchar_t name[128];
  swprintf_s(name, L"\\\\.\\pipe\\aura-bootstrap-fixture-%lu", GetCurrentProcessId());
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return 2;
  BYTE user[sizeof(TOKEN_USER) + SECURITY_MAX_SID_SIZE]; DWORD size = 0;
  bool got_user = GetTokenInformation(token, TokenUser, user, sizeof(user), &size) != FALSE;
  CloseHandle(token);
  if (!got_user) return 2;
  LPWSTR sid = nullptr;
  if (!ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(user)->User.Sid, &sid)) return 2;
  std::wstring sddl = L"D:P(A;;0x0012019b;;;"; sddl += sid; sddl += L")";
  LocalFree(sid);
  PSECURITY_DESCRIPTOR descriptor = nullptr;
  if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1,
                                                            &descriptor, nullptr)) return 2;
  SECURITY_ATTRIBUTES attributes = {sizeof(attributes), descriptor, FALSE};
  HANDLE server = CreateNamedPipeW(name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED |
      FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
      2, 8192, 8192, 0, &attributes);
  if (server == INVALID_HANDLE_VALUE) { LocalFree(descriptor); return 2; }
  int failures = 0;
  if (mode == L"pipe") {
    HANDLE client = CreateFileW(name, kEnvBoxPipeClientAccess, 0, nullptr, OPEN_EXISTING,
                                FILE_FLAG_OVERLAPPED, nullptr);
    failures += Check(client != INVALID_HANDLE_VALUE, "specific_client_access");
    if (client != INVALID_HANDLE_VALUE) {
      failures += Check(!EnvBoxValidateTrustedServicePipe(client), "ordinary_server_rejected");
      CloseHandle(client);
    }
    SetLastError(ERROR_SUCCESS);
    HANDLE second = CreateNamedPipeW(name, PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
        PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 2, 8192, 8192, 0, &attributes);
    DWORD error = GetLastError();
    failures += Check(second == INVALID_HANDLE_VALUE && error == ERROR_ACCESS_DENIED,
                       "additional_server_instance_denied");
    if (second != INVALID_HANDLE_VALUE) CloseHandle(second);
  } else if (argc == 3 && mode.compare(0, 9, L"fallback-") == 0) {
    Profile();
    if (mode == L"fallback-trusted") SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L"1");
    if (mode == L"fallback-invalid") SetEnvironmentVariableW(L"ENVBOX_TRUSTED_SERVICE_BOOTSTRAP", L"0");
    SetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", mode == L"fallback-legacy" ? L"aura-missing-fixture-pipe" : name);
    HMODULE module = LoadLibraryW(argv[2]);
    DWORD error = GetLastError();
    printf("runtime_loaded=%d error=%lu\n", module != nullptr, error);
    failures += Check(mode == L"fallback-legacy" ? module != nullptr : module == nullptr,
                       "production_environment_fallback_policy");
    if (module) FreeLibrary(module);
  } else failures = 1;
  CloseHandle(server); LocalFree(descriptor);
  return failures ? 1 : 0;
}
