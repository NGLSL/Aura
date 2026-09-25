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
int EnvBoxInstallGeoHooks();
int EnvBoxInstallLocaleHooks();
int EnvBoxInstallLanguageHooks();
int EnvBoxInstallProcessHooks();
int EnvBoxInstallDnsHooks();
int EnvBoxInstallRegistryHooks();
// Network Guard (ticket 56): Strict UDP deny for this process tree.
int EnvBoxInstallNetworkHooks();
