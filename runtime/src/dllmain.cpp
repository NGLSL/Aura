// EnvBox Runtime - domain hooks (timezone / geo / locale / UI lang / DNS / process).
// Policy: Fail Open on non-critical hook errors; complete init failure and
// incomplete Profile DNS / Strict Network Guard are fatal (Startup Fail Policy - no silent
// downgrade). Never call SetDynamicTimeZoneInformation; never hook real-time APIs.

#include <windows.h>

#include <stdio.h>

#include "audit.h"
#include "detours.h"
#include "hooks.h"
#include "runtime_profile.h"
#include "ipc_bootstrap.h"
#include "startup_gate.h"

static int g_hook_counts[10] = {};

static int InstallAllHooks() {
  DetourTransactionBegin();
  DetourUpdateThread(GetCurrentThread());

  // Optional environment hooks are independent. Profile DNS, Strict Network
  // Guard and an explicitly requested entry gate require complete install.
  int ok = 0;
  ok += g_hook_counts[0] = EnvBoxInstallTimeHooks();
  ok += g_hook_counts[1] = EnvBoxInstallWinRtTimeHooks();
  ok += g_hook_counts[2] = EnvBoxInstallGeoHooks();
  ok += g_hook_counts[3] = EnvBoxInstallLocaleHooks();
  ok += g_hook_counts[4] = EnvBoxInstallCrtLocaleHooks();
  ok += g_hook_counts[5] = EnvBoxInstallLanguageHooks();
  ok += g_hook_counts[6] = EnvBoxInstallDnsHooks();
  if (EnvBoxProfile()->dns_mode == 1 && !EnvBoxDnsHooksReady()) {
    DetourTransactionAbort();
    return 0;
  }
  ok += g_hook_counts[7] = EnvBoxInstallRegistryHooks();
  ok += g_hook_counts[8] = EnvBoxInstallProcessHooks();
  // Strict Network Guard is NOT Fail Open: incomplete attach must abort the
  // process (Startup Fail Policy) instead of silently running without UDP deny.
  if (!(g_hook_counts[9] = EnvBoxInstallNetworkHooks())) {
    DetourTransactionAbort();
    return 0;
  }

  if (!EnvBoxInstallStartupGate()) {
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
              "EnvBox Runtime hooks attached=%d (required policies validated)\n", ok);
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
    if (!EnvBoxRecoveryJobOpen()) return FALSE;
    EnvBoxAuditInit(EnvBoxProfile());
    if (!InstallAllHooks()) {
      // Startup Fail Policy: never run with Strict Network Guard unarmed.
      EnvBoxRecoveryJobClose();
      return FALSE;
    }
    SetEnvironmentVariableA("ENVBOX_RUNTIME_LOADED", "1");
    if (!EnvBoxIpcNotifyRuntimeIdentity(instance, g_hook_counts, 10)) {
      EnvBoxRecoveryJobClose();
      return FALSE;
    }
  } else if (reason == DLL_PROCESS_DETACH) {
    EnvBoxAuditShutdown();
    EnvBoxRecoveryJobClose();
  }
  return TRUE;
}

// Detours launches the opposite-bitness rundll32 with `<runtime>,#1` when a
// hooked process creates a child of the other architecture. Ordinal 1 must be
// this CALLBACK entry point in both Runtime DLLs (see the architecture-specific
// module definition files).
extern "C" void CALLBACK EnvBoxDetourFinishHelperProcess(
    HWND window, HINSTANCE instance, LPSTR command_line, int show) {
  DetourFinishHelperProcess(window, instance, command_line, show);
}

extern "C" const char* EnvBoxRuntimeLoaded() {
  return "EnvBox Runtime Loaded";
}
