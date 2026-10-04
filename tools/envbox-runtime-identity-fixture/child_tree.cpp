#include <windows.h>

static wchar_t target[32768], marker[32768], environment[65536], command[32768];
static char ansi_target[32768], ansi_command[32768];
static STARTUPINFOW startup;
static STARTUPINFOA startup_a;
static PROCESS_INFORMATION child;
static void WriteMarker(const wchar_t* name) {
  static wchar_t path[32768];
  if (!GetEnvironmentVariableW(name, path, 32768)) return;
  HANDLE file = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, 0, nullptr);
  if (file != INVALID_HANDLE_VALUE) {
    DWORD written; WriteFile(file, "entry", 5, &written, nullptr); CloseHandle(file);
  }
}
static size_t Copy(wchar_t* out, const wchar_t* value) {
  size_t n = 0; do { out[n] = value[n]; } while (value[n++]); return n;
}
static bool DuplicateChildBinding() {
  static char pipe_name[256], request[1024], response[256], instance[64], profile[64];
  if (!GetEnvironmentVariableA("ENVBOX_IPC_PIPE", pipe_name, 256) ||
      !GetEnvironmentVariableA("ENVBOX_INSTANCE_ID", instance, 64) ||
      !GetEnvironmentVariableA("ENVBOX_PROFILE_ID", profile, 64)) return false;
  FILETIME parent_time, child_time, exited, kernel, user;
  if (!GetProcessTimes(GetCurrentProcess(), &parent_time, &exited, &kernel, &user) ||
      !GetProcessTimes(child.hProcess, &child_time, &exited, &kernel, &user)) return false;
  size_t n = 0;
  auto text = [&](const char* value) { while (*value) request[n++] = *value++; };
  auto number = [&](ULONGLONG value) {
    char digits[32]; unsigned count = 0;
    do { digits[count++] = static_cast<char>('0' + value % 10); value /= 10; } while (value);
    while (count) request[n++] = digits[--count];
  };
  text("REGISTER_CHILD pid="); number(GetCurrentProcessId());
  text(" creation_time="); number((static_cast<ULONGLONG>(parent_time.dwHighDateTime) << 32) | parent_time.dwLowDateTime);
  text(" instance_id="); text(instance); text(" profile_id="); size_t profile_start = n; text(profile);
  text(" child_pid="); number(child.dwProcessId);
  text(" child_creation_time="); number((static_cast<ULONGLONG>(child_time.dwHighDateTime) << 32) | child_time.dwLowDateTime);
  request[n++] = '\n'; request[n] = 0;
  HANDLE pipe = INVALID_HANDLE_VALUE;
  ULONGLONG deadline = GetTickCount64() + 1000;
  do {
    WaitNamedPipeA(pipe_name, 25);
    pipe = CreateFileA(pipe_name, GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
    if (pipe == INVALID_HANDLE_VALUE) Sleep(1);
  } while (pipe == INVALID_HANDLE_VALUE && GetTickCount64() < deadline);
  if (pipe == INVALID_HANDLE_VALUE) return false;
  DWORD written, received;
  bool ok = true;
  char generation_digit = request[n - 2], profile_digit = request[profile_start];
  for (int step = 0; step < 3 && ok; ++step) {
    request[n - 2] = step == 0 ? (generation_digit == '9' ? '0' : generation_digit + 1) : generation_digit;
    request[profile_start] = step == 1 ? 'z' : profile_digit;
    ok = WriteFile(pipe, request, static_cast<DWORD>(n), &written, nullptr) && written == n &&
         ReadFile(pipe, response, 255, &received, nullptr) && received > 6;
    const char* expected = step < 2 ? "ERROR " : "CHILD_BOUND ";
    if (ok) for (unsigned i = 0; expected[i]; ++i) if (response[i] != expected[i]) ok = false;
  }
  CloseHandle(pipe);
  return ok;
}
extern "C" DWORD WINAPI FixtureEntry(void*) {
  wchar_t role[8] = {}, api[16] = {}, suspended[8] = {};
  GetEnvironmentVariableW(L"AURA_CHILD_ROLE", role, 8);
  if (role[0] == L'1') {
    static wchar_t value[64], locale[LOCALE_NAME_MAX_LENGTH];
    GetEnvironmentVariableW(L"AURA_IDENTITY_VALUE", value, 64);
    GetUserDefaultLocaleName(locale, LOCALE_NAME_MAX_LENGTH);
    const wchar_t* expected = L"profile-value";
    size_t i = 0; while (value[i] && value[i] == expected[i]) ++i;
    if (value[i] != expected[i] || locale[0] != L'e' || locale[1] != L'n' || locale[2] != L'-' || locale[3] != L'U' || locale[4] != L'S' || locale[5] != 0) ExitProcess(15);
    WriteMarker(L"AURA_CHILD_MARKER");
    Sleep(500);
    ExitProcess(0);
  }
  if (!GetEnvironmentVariableW(L"AURA_CHILD_TARGET", target, 32768) ||
      !GetEnvironmentVariableW(L"AURA_CHILD_MARKER", marker, 32768)) ExitProcess(10);
  GetEnvironmentVariableW(L"AURA_CHILD_API", api, 16);
  GetEnvironmentVariableW(L"AURA_CHILD_SUSPENDED", suspended, 8);
  size_t n = Copy(environment, L"AURA_CHILD_ROLE=1");
  // Gate fixtures deliberately try to disable their parent's gate. A normal
  // legacy child omits the internal gate flag; explicit "0" is unsupported.
  wchar_t legacy[8] = {};
  if (!GetEnvironmentVariableW(L"AURA_CHILD_LEGACY", legacy, 8))
    n += Copy(environment + n, L"ENVBOX_STARTUP_GATE=0");
  size_t key = Copy(environment + n, L"AURA_CHILD_MARKER=") - 1;
  n += key + Copy(environment + n + key, marker);
  environment[n] = 0;
  command[0] = L'"'; size_t length = Copy(command + 1, target) - 1;
  command[length + 1] = L'"'; command[length + 2] = 0;
  DWORD flags = CREATE_UNICODE_ENVIRONMENT | (suspended[0] == L'1' ? CREATE_SUSPENDED : 0);
  startup.cb = sizeof(startup); startup_a.cb = sizeof(startup_a);
  BOOL ok;
  if (api[0] == L'A' && api[1] == 0) {
    WideCharToMultiByte(CP_ACP, 0, target, -1, ansi_target, 32768, nullptr, nullptr);
    WideCharToMultiByte(CP_ACP, 0, command, -1, ansi_command, 32768, nullptr, nullptr);
    ok = CreateProcessA(ansi_target, ansi_command, nullptr, nullptr, FALSE, flags,
                         environment, nullptr, &startup_a, &child);
  } else if (api[0] == L'U') {
    HANDLE token;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY, &token)) ExitProcess(11);
    ok = CreateProcessAsUserW(token, target, command, nullptr, nullptr, FALSE, flags,
                              environment, nullptr, &startup, &child);
    CloseHandle(token);
  } else {
    ok = CreateProcessW(target, command, nullptr, nullptr, FALSE, flags,
                        environment, nullptr, &startup, &child);
  }
  if (!ok) ExitProcess(GetLastError());
  // An explicit protocol retry must return the same binding, rather than
  // claiming a second instance or changing the child's immutable snapshot.
  if (!DuplicateChildBinding()) { TerminateProcess(child.hProcess, 16); ExitProcess(16); }
  if (suspended[0] == L'1') {
    Sleep(100);
    if (GetFileAttributesW(marker) != INVALID_FILE_ATTRIBUTES) ExitProcess(12);
    if (ResumeThread(child.hThread) == static_cast<DWORD>(-1)) ExitProcess(13);
  }
  if (WaitForSingleObject(child.hProcess, 10000) != WAIT_OBJECT_0) { TerminateProcess(child.hProcess, 14); ExitProcess(14); }
  DWORD code; GetExitCodeProcess(child.hProcess, &code);
  CloseHandle(child.hThread); CloseHandle(child.hProcess);
  ExitProcess(code);
}
