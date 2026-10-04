// Public CreateProcessW regression fixture. Run --check for an uninjected Host
// control, or --inject <matching-runtime.dll> for actual Detours injection.
#include <windows.h>
#include <detours.h>
#include <cstdio>
#include <string>
#include "fault.h"

static void SetProfile() {
  SetEnvironmentVariableW(L"ENVBOX_PROFILE_ID", L"00000000-0000-4000-8000-000000000001");
  SetEnvironmentVariableW(L"ENVBOX_INSTANCE_ID", L"00000000-0000-4000-8000-000000000002");
  SetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", nullptr);
  SetEnvironmentVariableW(L"ENVBOX_LOCALE_NAME", L"en-US");
  SetEnvironmentVariableW(L"ENVBOX_UI_LANGUAGE", L"en-US");
  SetEnvironmentVariableW(L"ENVBOX_REGION", L"US");
  SetEnvironmentVariableW(L"ENVBOX_TZ_WINDOWS", L"Pacific Standard Time");
  SetEnvironmentVariableW(L"ENVBOX_TZ_IANA", L"America/Los_Angeles");
  SetEnvironmentVariableW(L"ENVBOX_INHERIT_CHILDREN", L"1");
  SetEnvironmentVariableW(L"ENVBOX_AUDIT", L"1");
  SetEnvironmentVariableW(L"ENVBOX_WEBRTC_POLICY", L"host");
}

static std::string Ansi(const wchar_t* path) {
  int length = WideCharToMultiByte(CP_ACP, 0, path, -1, nullptr, 0, nullptr, nullptr);
  std::string value(length, '\0');
  WideCharToMultiByte(CP_ACP, 0, path, -1, value.data(), length, nullptr, nullptr);
  return value;
}

static std::wstring Self() {
  wchar_t path[32768] = {};
  GetModuleFileNameW(nullptr, path, 32768);
  return path;
}

static bool RuntimeLoaded() {
#ifdef _WIN64
  return GetModuleHandleW(L"envbox-runtime64.dll") != nullptr;
#else
  return GetModuleHandleW(L"envbox-runtime32.dll") != nullptr;
#endif
}

static bool Cycle() {
  HANDLE event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (event == nullptr) return false;
  // The output buffer is not an ownership transfer on failed CreateProcess.
  // It may contain a caller-owned live handle from a previous operation.
  PROCESS_INFORMATION failed = {};
  failed.hProcess = event;
  failed.hThread = event;
  STARTUPINFOW startup = {};
  startup.cb = sizeof(startup);
  auto nonexistent = Self() + L".not-an-executable";
  BOOL created = CreateProcessW(nonexistent.c_str(), nullptr, nullptr, nullptr,
                               FALSE, 0, nullptr, nullptr, &startup, &failed);
  DWORD error = GetLastError();
  DWORD flags = 0;
  bool kept_handle = GetHandleInformation(event, &flags) != FALSE;
  if (!created && error == ERROR_FILE_NOT_FOUND && kept_handle) {
    CloseHandle(event);
  } else {
    std::printf("FAIL missing-image created=%d error=%lu caller-handle-valid=%d\n",
                created, error, kept_handle);
    if (kept_handle) CloseHandle(event);
    return false;
  }

  auto command = L"\"" + Self() + L"\" --child";
  PROCESS_INFORMATION child = {};
  if (!CreateProcessW(nullptr, command.data(), nullptr, nullptr, FALSE, 0,
                      nullptr, nullptr, &startup, &child)) {
    std::printf("FAIL success-cycle error=%lu\n", GetLastError());
    return false;
  }
  DWORD wait = WaitForSingleObject(child.hProcess, 10000);
  DWORD exit_code = 0;
  bool done = wait == WAIT_OBJECT_0 && GetExitCodeProcess(child.hProcess, &exit_code);
  if (!done) TerminateProcess(child.hProcess, 99);
  CloseHandle(child.hThread);
  CloseHandle(child.hProcess);
  if (!done || exit_code != (RuntimeLoaded() ? 42u : 43u)) {
    std::printf("FAIL child wait=%lu exit=%lu\n", wait, exit_code);
    return false;
  }
  return true;
}

int wmain(int argc, wchar_t** argv) {
  if (argc >= 2 && wcscmp(argv[1], L"--child") == 0) {
    return RuntimeLoaded() ? 42 : 43;
  }
  if (argc == 3 && wcscmp(argv[1], L"--inject") == 0) {
    auto config_root = Self() + L".config";
    SetEnvironmentVariableW(L"ENVBOX_CONFIG_ROOT", config_root.c_str());
    SetEnvironmentVariableW(L"ENVBOX_PROFILE_ID", L"00000000-0000-4000-8000-000000000001");
    SetEnvironmentVariableW(L"ENVBOX_INSTANCE_ID", L"00000000-0000-4000-8000-000000000002");
    SetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", nullptr);
    SetEnvironmentVariableW(L"ENVBOX_LOCALE_NAME", L"en-US");
    SetEnvironmentVariableW(L"ENVBOX_UI_LANGUAGE", L"en-US");
    SetEnvironmentVariableW(L"ENVBOX_REGION", L"US");
    SetEnvironmentVariableW(L"ENVBOX_TZ_WINDOWS", L"Pacific Standard Time");
    SetEnvironmentVariableW(L"ENVBOX_TZ_IANA", L"America/Los_Angeles");
    SetEnvironmentVariableW(L"ENVBOX_INHERIT_CHILDREN", L"1");
    SetEnvironmentVariableW(L"ENVBOX_AUDIT", L"1");
    SetEnvironmentVariableW(L"ENVBOX_WEBRTC_POLICY", L"host");
    auto command = L"\"" + Self() + L"\" --check-injected";
    auto path = std::wstring(argv[2]);
    int length = WideCharToMultiByte(CP_ACP, 0, path.c_str(), -1, nullptr, 0, nullptr, nullptr);
    std::string dll(length, '\0');
    WideCharToMultiByte(CP_ACP, 0, path.c_str(), -1, dll.data(), length, nullptr, nullptr);
    STARTUPINFOW startup = {};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION child = {};
    if (!DetourCreateProcessWithDllExW(nullptr, command.data(), nullptr, nullptr,
                                      FALSE, 0, nullptr, nullptr,
                                      &startup, &child, dll.c_str(), nullptr)) {
      std::printf("FAIL fixture injection error=%lu\n", GetLastError());
      return 2;
    }
    DWORD wait = WaitForSingleObject(child.hProcess, 120000);
    DWORD exit_code = 0;
    GetExitCodeProcess(child.hProcess, &exit_code);
    if (wait != WAIT_OBJECT_0) TerminateProcess(child.hProcess, 99);
    CloseHandle(child.hThread);
    CloseHandle(child.hProcess);
    return wait == WAIT_OBJECT_0 ? static_cast<int>(exit_code) : 3;
  }
  if (argc == 5 && wcscmp(argv[1], L"--fault-inject") == 0) {
    SetProfile();
    auto config_root = Self() + L".config";
    SetEnvironmentVariableW(L"ENVBOX_CONFIG_ROOT", config_root.c_str());
    SetEnvironmentVariableW(L"ENVBOX_CLEANUP_TARGET", Self().c_str());
    auto command = L"\"" + Self() + L"\" --fault-body " + argv[4];
    auto runtime = Ansi(argv[2]);
    auto fault = Ansi(argv[3]);
    LPCSTR dlls[] = {fault.c_str(), runtime.c_str()};
    STARTUPINFOW startup = {};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION child = {};
    if (!DetourCreateProcessWithDllsW(nullptr, command.data(), nullptr, nullptr,
                                     FALSE, 0, nullptr, nullptr, &startup, &child,
                                     2, dlls, nullptr)) return 11;
    DWORD wait = WaitForSingleObject(child.hProcess, 120000);
    DWORD exit_code = 0;
    GetExitCodeProcess(child.hProcess, &exit_code);
    if (wait != WAIT_OBJECT_0) {
      TerminateProcess(child.hProcess, 99);
      WaitForSingleObject(child.hProcess, 5000);
    }
    CloseHandle(child.hThread);
    CloseHandle(child.hProcess);
    return wait == WAIT_OBJECT_0 ? static_cast<int>(exit_code) : 12;
  }
  if (argc == 3 && wcscmp(argv[1], L"--fault-body") == 0) {
#ifdef _WIN64
    HMODULE fault = GetModuleHandleW(L"cleanup-fault64.dll");
#else
    HMODULE fault = GetModuleHandleW(L"cleanup-fault32.dll");
#endif
    if (!RuntimeLoaded() || fault == nullptr) return 13;
    auto set_mode = reinterpret_cast<CleanupFixtureSetModeFn>(GetProcAddress(fault, "CleanupFixtureSetMode"));
    auto finish = reinterpret_cast<CleanupFixtureFinishFn>(GetProcAddress(fault, "CleanupFixtureFinish"));
    if (!set_mode || !finish) return 14;
    int fault_mode = _wtoi(argv[2]);
    DWORD before = 0, after = 0;
    set_mode(0);
    if (!Cycle()) return 15;
    finish();
    GetProcessHandleCount(GetCurrentProcess(), &before);
    for (int i = 0; i < 32; ++i) {
      set_mode(fault_mode);
      auto command = L"\"" + Self() + L"\" --child";
      STARTUPINFOW startup = {};
      startup.cb = sizeof(startup);
      PROCESS_INFORMATION child = {};
      BOOL created = CreateProcessW(nullptr, command.data(), nullptr, nullptr,
                                    FALSE, 0, nullptr, nullptr, &startup, &child);
      DWORD error = GetLastError();
      if (created) {
        // Regression cleanup is limited to the exact fixture-owned handles.
        TerminateProcess(child.hProcess, 99);
        WaitForSingleObject(child.hProcess, 5000);
        CloseHandle(child.hThread);
        CloseHandle(child.hProcess);
      }
      CleanupFixtureReport r = finish();
      bool good = !created && error == r.fault_error && error != 0 &&
                  r.pid != 0 && r.process_closes == 1 && r.thread_closes == 1 &&
                  r.terminate_calls == 1 && r.wait_result == WAIT_OBJECT_0 &&
                  r.forced_cleanup == 0;
      if (!good) {
        std::printf("FAIL fault=%d created=%d returned-error=%lu native-error=%lu "
                    "pid=%lu process-closes=%lu thread-closes=%lu terminate=%lu "
                    "wait=%lu exit=%lu forced-cleanup=%lu\n", fault_mode, created,
                    error, r.fault_error, r.pid, r.process_closes, r.thread_closes,
                    r.terminate_calls, r.wait_result, r.exit_code, r.forced_cleanup);
        return 16;
      }
      set_mode(0);
      if (!Cycle()) return 17;
      finish();
    }
    GetProcessHandleCount(GetCurrentProcess(), &after);
    std::printf("fault=%d cycles=32 native-error-preserved=true ownership=single "
                "child-exited=true handles-before=%lu handles-after=%lu\n",
                fault_mode, before, after);
    return before == after ? 0 : 18;
  }
  if (argc != 2 || (wcscmp(argv[1], L"--check") != 0 &&
                    wcscmp(argv[1], L"--check-injected") != 0)) {
    std::printf("invalid fixture arguments argc=%d\n", argc);
    for (int i = 0; i < argc; ++i) std::wprintf(L"arg[%d]=%ls\n", i, argv[i]);
    return 4;
  }
  bool injected = wcscmp(argv[1], L"--check-injected") == 0;
  if (RuntimeLoaded() != injected) return 5;
  if (!Cycle()) return 6;
  DWORD before = 0, after = 0;
  if (!GetProcessHandleCount(GetCurrentProcess(), &before)) return 7;
  for (int i = 0; i < 32; ++i) if (!Cycle()) return 8;
  if (!GetProcessHandleCount(GetCurrentProcess(), &after)) return 9;
  std::printf("runtime=%d cycles=33 handles-before=%lu handles-after=%lu\n",
              injected, before, after);
  return before == after ? 0 : 10;
}
