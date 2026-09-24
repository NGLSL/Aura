// Immutable Runtime Profile loaded once at DLL init (ticket 05).
// Values come from profiles.toml selected by ENVBOX_PROFILE_ID.

#pragma once

#include <windows.h>

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
