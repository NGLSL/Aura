// EnvBox Runtime - ticket 04: Detours inject smoke (no API hooks yet).
// Policy: Fail Open for future hooks; injection failure is handled by the launcher.
// Complete Runtime init failure (no Profile identity) is fatal per Startup Fail Policy.

#include <windows.h>

#include <stdio.h>

#include "detours.h"

// Ticket 04 "Load Profile": resolve Profile identity from the Environment Block
// and verify it against profiles.toml when ENVBOX_CONFIG_ROOT is present.
// Immutable RuntimeProfile field parse + Install Hooks land in ticket 05+.
struct LoadedProfile {
  char profile_id[64];
  char instance_id[64];
  int loaded;
};

static LoadedProfile g_profile = {};

static int ReadEnvId(const char* name, char* buf, DWORD cap) {
  DWORD n = GetEnvironmentVariableA(name, buf, cap);
  if (n == 0 || n >= cap) {
    buf[0] = '\0';
    return 0;
  }
  return 1;
}

static int ProfileIdInProfilesToml(const char* profile_id) {
  char root[MAX_PATH];
  DWORD n = GetEnvironmentVariableA("ENVBOX_CONFIG_ROOT", root, (DWORD)sizeof(root));
  if (n == 0 || n >= sizeof(root)) {
    return 1;  // no store path in env; Environment Block IDs are authoritative
  }
  char path[MAX_PATH + 32];
  _snprintf_s(path, sizeof(path), _TRUNCATE, "%s\\profiles.toml", root);
  HANDLE file = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    return 1;  // missing store: accept Environment Block identity
  }
  char data[65536];
  DWORD read = 0;
  BOOL ok = ReadFile(file, data, (DWORD)sizeof(data) - 1, &read, nullptr);
  CloseHandle(file);
  if (!ok) {
    return 0;
  }
  data[read] = '\0';
  return strstr(data, profile_id) != nullptr;
}

// Returns 1 on success (profile loaded), 0 on fatal init failure.
static int LoadProfileFromEnv() {
  if (!ReadEnvId("ENVBOX_PROFILE_ID", g_profile.profile_id,
                 (DWORD)sizeof(g_profile.profile_id))) {
    return 0;
  }
  ReadEnvId("ENVBOX_INSTANCE_ID", g_profile.instance_id,
            (DWORD)sizeof(g_profile.instance_id));
  if (!ProfileIdInProfilesToml(g_profile.profile_id)) {
    return 0;
  }

  char msg[256];
  _snprintf_s(msg, sizeof(msg), _TRUNCATE,
              "EnvBox Runtime Loaded profile=%s instance=%s (hooks deferred)\n",
              g_profile.profile_id, g_profile.instance_id);
  OutputDebugStringA(msg);
  SetEnvironmentVariableA("ENVBOX_RUNTIME_LOADED", "1");
  g_profile.loaded = 1;
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
    if (!LoadProfileFromEnv()) {
      // Startup Fail Policy: do not run without a Profile.
      return FALSE;
    }
  }
  return TRUE;
}

extern "C" __declspec(dllexport) const char* EnvBoxRuntimeLoaded() {
  return "EnvBox Runtime Loaded";
}
