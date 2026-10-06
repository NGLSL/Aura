// Shared hook install helpers (ticket 05+). Fail Open: individual attach failures
// do not abort the process; only complete init failure is fatal (DllMain).

#pragma once

#include "runtime_profile.h"

#include "detours.h"

// Attach one detour inside the current transaction. Returns 1 on success.
// On failure the original pointer is left unchanged (Fail Open for that API).
template <typename T>
static inline int EnvBoxAttach(T* pp, T hook) {
  LONG err = DetourAttach(reinterpret_cast<PVOID*>(pp), reinterpret_cast<PVOID>(hook));
  return err == NO_ERROR;
}

// Per-domain install entry points (split modules).
int EnvBoxInstallTimeHooks();
int EnvBoxInstallWinRtTimeHooks();
int EnvBoxInstallGeoHooks();
int EnvBoxInstallLocaleHooks();
// UCRT's setlocale/_wsetlocale resolve their locale defaults internally and
// do not necessarily pass through the Win32 NLS entry points above.  These
// hooks only virtualize the empty-locale request; explicit locale names and
// query calls remain untouched.
int EnvBoxInstallCrtLocaleHooks();
int EnvBoxInstallLanguageHooks();
// Returns the attached count, or -1 for an invalid startup policy flag.
int EnvBoxInstallProcessHooks();
// Latched once during process-hook installation, before application entry.
// Target mutations of ENVBOX_STARTUP_GATE cannot change this policy.
bool EnvBoxControlledStartup();
int EnvBoxInstallDnsHooks();
int EnvBoxDnsHooksReady();
int EnvBoxInstallRegistryHooks();
// Network Guard (ticket 56): Strict UDP deny for this process tree.
// Returns 1 when the process may start (non-Strict, or Strict guard armed).
// Returns 0 when Strict Network Guard cannot be armed - caller must Startup
// Fail (never run without the UDP deny / never downgrade to Balanced).
int EnvBoxInstallNetworkHooks();
