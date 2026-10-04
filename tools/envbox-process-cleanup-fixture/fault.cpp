// TEST ONLY: never link this module into a product or Runtime bundle.
#include "fault.h"
#include <detours.h>
#include <cstdio>
#include <string>

static decltype(&CreateProcessW) TrueCreate = CreateProcessW;
static decltype(&ResumeThread) TrueResume = ResumeThread;
static decltype(&CloseHandle) TrueClose = CloseHandle;
static decltype(&TerminateProcess) TrueTerminate = TerminateProcess;
static decltype(&AssignProcessToJobObject) TrueAssign = AssignProcessToJobObject;
static int mode = 0;
static HANDLE process = nullptr, thread = nullptr, observer = nullptr;
static CleanupFixtureReport report = {};
static wchar_t target[32768] = {};

static void FinishOwnedChild() {
  if (observer == nullptr) return;
  report.wait_result = WaitForSingleObject(observer, 5000);
  GetExitCodeProcess(observer, &report.exit_code);
  if (report.wait_result != WAIT_OBJECT_0) {
    // A regression must not leave a test child behind. This duplicate refers
    // only to the child created by this fixture; no PID-based termination.
    report.forced_cleanup = 1;
    TrueTerminate(observer, 99);
    WaitForSingleObject(observer, 5000);
  }
  TrueClose(observer);
  observer = nullptr;
}

extern "C" void CleanupFixtureSetMode(int value) {
  FinishOwnedChild();
  report = {};
  process = thread = nullptr;
  mode = value;
}

extern "C" CleanupFixtureReport CleanupFixtureFinish() {
  FinishOwnedChild();
  return report;
}

static BOOL WINAPI SpyClose(HANDLE handle) {
  if (handle == process && process != nullptr) ++report.process_closes;
  if (handle == thread && thread != nullptr) ++report.thread_closes;
  return TrueClose(handle);
}

static BOOL WINAPI SpyTerminate(HANDLE handle, UINT code) {
  if (GetProcessId(handle) == report.pid && report.pid != 0) {
    ++report.terminate_calls;
  }
  return TrueTerminate(handle, code);
}

static DWORD WINAPI FaultResume(HANDLE handle) {
  if (mode != 2 || handle != thread) return TrueResume(handle);
  // Invoke the real API with a valid handle lacking THREAD_SUSPEND_RESUME.
  HANDLE query = nullptr;
  if (!DuplicateHandle(GetCurrentProcess(), handle, GetCurrentProcess(), &query,
                       THREAD_QUERY_LIMITED_INFORMATION, FALSE, 0)) return DWORD(-1);
  DWORD result = TrueResume(query);
  DWORD error = GetLastError();
  TrueClose(query);
  report.fault_error = error;
  SetLastError(error);
  return result;
}

static BOOL WINAPI FaultAssign(HANDLE job, HANDLE child) {
  if (mode != 3 || GetProcessId(child) != report.pid) return TrueAssign(job, child);
  BOOL result = TrueAssign(nullptr, child);
  report.fault_error = GetLastError();
  SetLastError(report.fault_error);
  return result;
}

static BOOL WINAPI SpyCreate(LPCWSTR app, LPWSTR command, LPSECURITY_ATTRIBUTES pa,
                            LPSECURITY_ATTRIBUTES ta, BOOL inherit, DWORD flags,
                            LPVOID env, LPCWSTR cwd, LPSTARTUPINFOW si,
                            LPPROCESS_INFORMATION pi) {
  if (mode == 1 && report.pid != 0) {
    // Prevent Detours' helper fallback from repairing the deliberately dead
    // child. This is a real Windows missing-image failure, not a fake code.
    BOOL result = TrueCreate(L"Z:\\aura-cleanup-fixture-missing-helper.exe", nullptr,
                             pa, ta, inherit, flags, env, cwd, si, pi);
    report.fault_error = GetLastError();
    SetLastError(report.fault_error);
    return result;
  }
  BOOL result = TrueCreate(app, command, pa, ta, inherit, flags, env, cwd, si, pi);
  DWORD error = GetLastError();
  bool ours = target[0] != L'\0' &&
              ((app && _wcsicmp(app, target) == 0) || (command && wcsstr(command, target)));
  if (result && ours && pi != nullptr) {
    process = pi->hProcess;
    thread = pi->hThread;
    report.pid = pi->dwProcessId;
    DuplicateHandle(GetCurrentProcess(), process, GetCurrentProcess(), &observer,
                    PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE | PROCESS_TERMINATE,
                    FALSE, 0);
    if (mode == 1) {
      TrueTerminate(process, 98);
      WaitForSingleObject(process, 5000);
    }
  }
  SetLastError(error);
  return result;
}

extern "C" void CALLBACK CleanupFixtureFinishHelper(HWND w, HINSTANCE i,
                                                     LPSTR c, int s) {
  DetourFinishHelperProcess(w, i, c, s);
}

static void WriteReport() {
  wchar_t path[32768] = {};
  if (GetEnvironmentVariableW(L"ENVBOX_CLEANUP_REPORT", path, 32768) == 0) return;
  FinishOwnedChild();
  HANDLE file = CreateFileW(path, GENERIC_WRITE, FILE_SHARE_READ, nullptr,
                            CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return;
  char text[512];
  int length = sprintf_s(text, "pid=%lu process_closes=%lu thread_closes=%lu "
      "terminate_calls=%lu fault_error=%lu wait=%lu exit=%lu forced_cleanup=%lu\n",
      report.pid, report.process_closes, report.thread_closes, report.terminate_calls,
      report.fault_error, report.wait_result, report.exit_code, report.forced_cleanup);
  DWORD written = 0;
  WriteFile(file, text, length, &written, nullptr);
  TrueClose(file);
}

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID) {
  if (DetourIsHelperProcess()) return TRUE;
  if (reason == DLL_PROCESS_ATTACH) {
    DisableThreadLibraryCalls(instance);
    DetourRestoreAfterWith();
    GetEnvironmentVariableW(L"ENVBOX_CLEANUP_TARGET", target, 32768);
    wchar_t value[8] = {};
    if (GetEnvironmentVariableW(L"ENVBOX_CLEANUP_MODE", value, 8)) mode = _wtoi(value);
    DetourTransactionBegin();
    DetourUpdateThread(GetCurrentThread());
    DetourAttach(reinterpret_cast<PVOID*>(&TrueCreate), SpyCreate);
    DetourAttach(reinterpret_cast<PVOID*>(&TrueResume), FaultResume);
    DetourAttach(reinterpret_cast<PVOID*>(&TrueClose), SpyClose);
    DetourAttach(reinterpret_cast<PVOID*>(&TrueTerminate), SpyTerminate);
    DetourAttach(reinterpret_cast<PVOID*>(&TrueAssign), FaultAssign);
    return DetourTransactionCommit() == NO_ERROR;
  }
  if (reason == DLL_PROCESS_DETACH) WriteReport();
  return TRUE;
}
