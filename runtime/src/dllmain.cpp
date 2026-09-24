// EnvBox Runtime - ticket 01 scaffold (empty implementation).
// Hook install lands with Profile injection tickets; Fail Open remains the policy.

#include <windows.h>

extern "C" BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID reserved) {
  (void)instance;
  (void)reserved;
  if (reason == DLL_PROCESS_ATTACH) {
    // DetourIsHelperProcess / DetourRestoreAfterWith come with Detours in ticket 04.
  }
  return TRUE;
}

extern "C" __declspec(dllexport) const char* EnvBoxRuntimeLoaded() {
  return "EnvBox Runtime Loaded";
}
