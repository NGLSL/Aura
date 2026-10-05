#include <windows.h>

// A no-CRT/no-TLS fixture isolates mixed-bitness process creation from shell
// quoting. The child inherits the actual immutable environment through Aura.
static wchar_t target[32768], command[32768];
static STARTUPINFOW startup;
static PROCESS_INFORMATION child;
static wchar_t exit_parent[8];
extern "C" DWORD WINAPI FixtureEntry(void*) {
  DWORD length = GetEnvironmentVariableW(L"AURA_RECOVERY_CHILD_TARGET", target, 32768);
  if (!length || length >= 32765) ExitProcess(10);
  command[0] = L'"';
  // Volatile stores prevent Release optimization from importing CRT memcpy.
  volatile wchar_t* destination = command;
  for (DWORD i = 0; i < length; ++i) destination[i + 1] = target[i];
  command[length + 1] = L'"'; command[length + 2] = 0;
  startup.cb = sizeof(startup);
  if (!CreateProcessW(target, command, nullptr, nullptr, FALSE, CREATE_NO_WINDOW,
                      nullptr, nullptr, &startup, &child)) ExitProcess(GetLastError());
  CloseHandle(child.hThread);
  GetEnvironmentVariableW(L"AURA_RECOVERY_PARENT_EXIT", exit_parent, 8);
  if (exit_parent[0] == L'1') { CloseHandle(child.hProcess); ExitProcess(0); }
  DWORD waited = WaitForSingleObject(child.hProcess, 30000);
  DWORD code = 11;
  if (waited == WAIT_OBJECT_0) GetExitCodeProcess(child.hProcess, &code);
  CloseHandle(child.hProcess);
  ExitProcess(code);
}
