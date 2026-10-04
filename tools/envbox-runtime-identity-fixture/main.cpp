#include <windows.h>
#include <stdio.h>
#include <wchar.h>

// Native x86/x64 public fixture: no Aura headers or identity encoder. Its
// observations independently confirm the effective application environment.
int main() {
  wchar_t value[128] = {};
  wchar_t locale[LOCALE_NAME_MAX_LENGTH] = {};
  GetEnvironmentVariableW(L"AURA_IDENTITY_VALUE", value, 128);
  GetUserDefaultLocaleName(locale, LOCALE_NAME_MAX_LENGTH);
  bool valid = wcscmp(value, L"profile-value") == 0 &&
               wcscmp(locale, L"en-US") == 0;
  printf("fixture_bits=%zu effective_locale=%ls config_value=%ls valid=%d\n",
         sizeof(void*) * 8, locale, value, valid ? 1 : 0);
  fflush(stdout);
  DWORD delay = GetEnvironmentVariableW(L"AURA_RECOVERY_FIXTURE", value, 128) ? 10000 : 3000;
  if (GetEnvironmentVariableW(L"AURA_SHORT_IDENTITY_FIXTURE", value, 128)) delay = 0;
  Sleep(delay);
  return valid ? 0 : 1;
}
