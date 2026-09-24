// UI Language hooks (ticket 05). Primary: GetUserDefaultUILanguage.

#include "hooks.h"

#include <wchar.h>

static LANGID(WINAPI* TrueGetUserDefaultUILanguage)() = GetUserDefaultUILanguage;

static LANGID ProfileUiLangId() {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || !pfl->has_ui) {
    return 0;
  }
  LCID lcid = LocaleNameToLCID(pfl->ui_language, 0);
  if (lcid == 0) {
    return 0;
  }
  return LANGIDFROMLCID(lcid);
}

static LANGID WINAPI HookGetUserDefaultUILanguage() {
  LANGID id = ProfileUiLangId();
  if (id != 0) {
    return id;
  }
  // Fail Open.
  return TrueGetUserDefaultUILanguage();
}

int EnvBoxInstallLanguageHooks() {
  return EnvBoxAttach(&TrueGetUserDefaultUILanguage, HookGetUserDefaultUILanguage);
}
