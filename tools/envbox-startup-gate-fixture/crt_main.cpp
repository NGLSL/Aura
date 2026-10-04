#include <windows.h>
#include <stdio.h>

// The standard MSVC CRT owns the PE entry. Marker is in application main/
// WinMain to test whether this ordinary startup is within the gate's scope.
static int ApplicationMain() {
  wchar_t path[1024];
  if (!GetEnvironmentVariableW(L"AURA_GATE_MARKER", path, 1024)) return 2;
  HANDLE file = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return 3;
  DWORD written;
  BOOL ok = WriteFile(file, "crt-entry", 9, &written, nullptr);
  CloseHandle(file);
  puts("standard CRT application entry reached");
  return ok && written == 9 ? 0 : 4;
}
#ifdef FIXTURE_GUI
int WINAPI WinMain(HINSTANCE, HINSTANCE, LPSTR, int) { return ApplicationMain(); }
#else
int main() { return ApplicationMain(); }
#endif
