// Immutable Runtime Profile loaded once at DLL init (V0.3 ticket 45).
//
// Values come from Runtime IPC Bootstrap (Broker PROFILE DTO) or, as Win32
// fallback, from ENVBOX_* structured value vars. C++ never parses profiles.toml.

#pragma once

#include <windows.h>

#ifndef ENVBOX_DNS_MAX
#define ENVBOX_DNS_MAX 8
#endif
#ifndef ENVBOX_REG_MAX
#define ENVBOX_REG_MAX 16
#endif

struct RuntimeProfile {
  wchar_t locale_name[85];
  wchar_t ui_language[85];
  wchar_t region[16];
  wchar_t tz_windows[128];
  wchar_t tz_iana[128];
  wchar_t profile_id[64];
  wchar_t instance_id[64];
  int has_locale;
  int has_ui;
  int has_region;
  int has_tz;
  int inherit_children;
  // Audit Mode (ticket 20): 0 = off (default), 1 = write JSONL sink.
  int audit;
  // DNS View (ticket 08): 0 = Host, 1 = VirtualView (DnsMode).
  int dns_mode;
  int dns_server_count;
  char dns_servers[ENVBOX_DNS_MAX][64];
  // Registry Virtual View (ticket 09): extra whitelist_paths from Profile.
  int registry_path_count;
  wchar_t registry_paths[ENVBOX_REG_MAX][128];
};

// Process-wide immutable profile after successful init. Never mutated later.
const RuntimeProfile* EnvBoxProfile();

// Returns 1 on success. Fails when ENVBOX_PROFILE_ID is missing or the
// profile cannot be resolved. Does not install hooks.
int EnvBoxLoadProfile();

// Lookup DYNAMIC_TIME_ZONE_INFORMATION by Windows ID (Profile timezone).
// Returns 1 and fills *out on success.
int EnvBoxLookupTimeZone(const wchar_t* windows_id,
                         DYNAMIC_TIME_ZONE_INFORMATION* out);

// Absolute path of this Runtime DLL (for child injection).
const char* EnvBoxRuntimeDllPathA();