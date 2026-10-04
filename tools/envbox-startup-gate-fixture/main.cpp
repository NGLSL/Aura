#include <windows.h>

// Deliberately independent of Aura and CRT. The marker is written at the
// actual PE entry, allowing both architectures to prove its ordering.
static void WriteMarker(const wchar_t* variable) {
  wchar_t path[1024];
  if (!GetEnvironmentVariableW(variable, path, 1024)) ExitProcess(2);
  HANDLE file = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) ExitProcess(3);
  DWORD written;
  WriteFile(file, "entry", 5, &written, nullptr);
  CloseHandle(file);
}
static void WriteProcessFacts() {
  wchar_t path[1024];
  if (!GetEnvironmentVariableW(L"AURA_GATE_PID_MARKER", path, 1024)) return;
  FILETIME created, exited, kernel, user;
  if (!GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user)) ExitProcess(4);
  ULONGLONG generation = (static_cast<ULONGLONG>(created.dwHighDateTime) << 32) | created.dwLowDateTime;
  char body[96];
  DWORD length = 0;
  ULONGLONG values[] = {GetCurrentProcessId(), generation};
  for (ULONGLONG value : values) {
    // Fixed hexadecimal avoids compiler CRT 64-bit division helpers on x86.
    for (int digit = 0; digit < 16; ++digit) {
      body[length++] = "0123456789abcdef"[value >> 60];
      value <<= 4;
    }
    body[length++] = ' ';
  }
  HANDLE file = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) ExitProcess(5);
  DWORD written;
  BOOL ok = WriteFile(file, body, length, &written, nullptr);
  CloseHandle(file);
  if (!ok || written != length) ExitProcess(6);
}
extern "C" DWORD WINAPI FixtureEntry(void*) {
  WriteMarker(L"AURA_GATE_MARKER");
  WriteProcessFacts();
  // Optional fixture-only lifetime for Job/Stop tests. Never wait beyond 30s,
  // and never make ordinary startup validation depend on this option.
  wchar_t duration[16];
  DWORD count = GetEnvironmentVariableW(L"AURA_GATE_WAIT_MS", duration, 16);
  DWORD milliseconds = 0;
  if (count > 0 && count < 16) {
    for (DWORD i = 0; i < count; ++i) {
      if (duration[i] < L'0' || duration[i] > L'9') { milliseconds = 0; break; }
      milliseconds = milliseconds * 10 + duration[i] - L'0';
      if (milliseconds > 30000) { milliseconds = 30000; break; }
    }
  }
  if (milliseconds > 0) Sleep(milliseconds);
#ifdef FIXTURE_RETURN
  return 0;
#else
  ExitProcess(0);
#endif
}

#ifdef FIXTURE_TLS
void NTAPI TlsCallback(void*, DWORD reason, void*) {
  if (reason == DLL_PROCESS_ATTACH) WriteMarker(L"AURA_TLS_MARKER");
}
#pragma section(".tls", long, read, write)
__declspec(allocate(".tls")) char tls_begin;
__declspec(allocate(".tls")) char tls_end;
DWORD tls_index;
PIMAGE_TLS_CALLBACK tls_callbacks[] = {TlsCallback, nullptr};
#pragma section(".rdata$T", long, read)
extern "C" __declspec(allocate(".rdata$T")) const IMAGE_TLS_DIRECTORY _tls_used = {
  reinterpret_cast<ULONG_PTR>(&tls_begin), reinterpret_cast<ULONG_PTR>(&tls_end),
  reinterpret_cast<ULONG_PTR>(&tls_index), reinterpret_cast<ULONG_PTR>(tls_callbacks), 0, 0
};
#ifdef _WIN64
#pragma comment(linker, "/INCLUDE:_tls_used")
#else
#pragma comment(linker, "/INCLUDE:__tls_used")
#endif
#endif
