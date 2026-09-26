// EnvBox Runtime - domain hooks (timezone / geo / locale / UI lang / DNS / process).
// Policy: Fail Open on non-critical hook errors; complete init failure and
// incomplete Strict Network Guard are fatal (Startup Fail Policy - no silent
// downgrade). Never call SetDynamicTimeZoneInformation; never hook real-time APIs.

#include <windows.h>

#include <stdio.h>

#include "audit.h"
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
  // Strict Network Guard is NOT Fail Open: incomplete attach must abort the
  // process (Startup Fail Policy) instead of silently running without UDP deny.
  if (!EnvBoxInstallNetworkHooks()) {
    DetourTransactionAbort();
    return 0;
  }

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
    EnvBoxAuditInit(EnvBoxProfile());
    if (!InstallAllHooks()) {
      // Startup Fail Policy: never run with Strict Network Guard unarmed.
      return FALSE;
    }
    SetEnvironmentVariableA("ENVBOX_RUNTIME_LOADED", "1");
  } else if (reason == DLL_PROCESS_DETACH) {
    EnvBoxAuditShutdown();
  }
  return TRUE;
}

extern "C" __declspec(dllexport) const char* EnvBoxRuntimeLoaded() {
  return "EnvBox Runtime Loaded";
}
