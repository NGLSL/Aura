// EnvBox Runtime - domain hooks (timezone / geo / locale / UI lang / DNS / process).
// Policy: Fail Open on hook errors; complete init failure is fatal (Startup Fail Policy).
// Never call SetDynamicTimeZoneInformation; never hook real-time APIs.

#include <windows.h>

#include <stdio.h>

#include "detours.h"
#include "hooks.h"
#include "runtime_profile.h"

static int InstallAllHooks() {
  DetourTransactionBegin();
  DetourUpdateThread(GetCurrentThread());

  // Each install is independent (Fail Open per API).
  int ok = 0;
  ok += EnvBoxInstallTimeHooks();
  ok += EnvBoxInstallGeoHooks();
  ok += EnvBoxInstallLocaleHooks();
  ok += EnvBoxInstallLanguageHooks();
  ok += EnvBoxInstallDnsHooks();
  ok += EnvBoxInstallRegistryHooks();
  ok += EnvBoxInstallProcessHooks();

  LONG err = DetourTransactionCommit();
  if (err != NO_ERROR) {
    // Commit already ended the transaction. Run un-hooked (Fail Open).
    return 0;
  }
  char msg[128];
  _snprintf_s(msg, sizeof(msg), _TRUNCATE,
              "EnvBox Runtime hooks attached=%d (Fail Open per API)\n", ok);
  OutputDebugStringA(msg);
  return 1;
}

extern "C" BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason,
                               LPVOID reserved) {
  (void)reserved;

  if (DetourIsHelperProcess()) {
    return TRUE;
  }

  if (reason == DLL_PROCESS_ATTACH) {
    DisableThreadLibraryCalls(instance);
    DetourRestoreAfterWith();
    if (!EnvBoxLoadProfile()) {
      // Startup Fail Policy: do not run without a Profile.
      return FALSE;
    }
    // Hooks may partially fail; process still starts (Fail Open).
    InstallAllHooks();
    SetEnvironmentVariableA("ENVBOX_RUNTIME_LOADED", "1");
  }
  return TRUE;
}

extern "C" __declspec(dllexport) const char* EnvBoxRuntimeLoaded() {
  return "EnvBox Runtime Loaded";
}
