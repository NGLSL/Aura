#include "runtime_profile.h"
#include <cstring>
#include <cwchar>
#include <string>
#include <vector>
extern "C" const char EnvBoxIdentityCapabilities[] =
    "version=1;computer_name=1;user_name=1;mac_address=1;machine_guid=1";
static bool AlphaNum(char c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
         (c >= '0' && c <= '9');
}
static int Hex(char c) {
  if (c >= '0' && c <= '9') return c - '0';
  if (c >= 'A' && c <= 'F') return c - 'A' + 10;
  return -1;
}
int EnvBoxDecodeIdentityConfiguration(RuntimeProfile *p,
                                      EnvBoxDnsFieldGetter get, void *ctx) {
  const char *keys[] = {"identity_computer_name", "identity_user_name",
                        "identity_mac_address", "identity_machine_guid"};
  wchar_t *fields[] = {p->identity_computer_name, p->identity_user_name,
                       p->identity_mac_address, p->identity_machine_guid};
  const size_t caps[] = {16, 65, 18, 37};
  for (int i = 0; i < 4; ++i) {
    fields[i][0] = 0;
    char text[65] = {};
    int found = get(ctx, keys[i], text, caps[i]);
    if (found < 0) return 0;
    if (!found) continue;
    size_t n = strlen(text);
    if (!n || n >= caps[i]) return 0;
    if (i == 0) {
      if (!AlphaNum(text[0]) || !AlphaNum(text[n - 1])) return 0;
      for (size_t j = 0; j < n; ++j)
        if (!AlphaNum(text[j]) && text[j] != '-') return 0;
    }
    if (i == 1) {
      for (size_t j = 0; j < n; ++j)
        if (!AlphaNum(text[j]) && text[j] != '.' && text[j] != '_' &&
            text[j] != '-')
          return 0;
    }
    if (i == 2) {
      if (n != 17) return 0;
      bool any = false, allff = true;
      for (int j = 0; j < 6; ++j) {
        if (j && text[j * 3 - 1] != ':') return 0;
        int a = Hex(text[j * 3]), b = Hex(text[j * 3 + 1]);
        if (a < 0 || b < 0) return 0;
        BYTE v = (BYTE)(a * 16 + b);
        p->identity_mac_bytes[j] = v;
        any |= v != 0;
        allff &= v == 255;
      }
      if (!any || allff || (p->identity_mac_bytes[0] & 1)) return 0;
    }
    if (i == 3) {
      if (n != 36) return 0;
      bool any = false;
      for (size_t j = 0; j < n; ++j) {
        bool dash = j == 8 || j == 13 || j == 18 || j == 23;
        char c = text[j];
        if (dash) {
          if (c != '-') return 0;
        } else {
          if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return 0;
          any |= c != '0';
        }
      }
      if (!any) return 0;
    }
    for (size_t j = 0; j <= n; ++j)
      fields[i][j] = (wchar_t)(unsigned char)text[j];
  }
  return 1;
}
int EnvBoxEmitIdentityConfiguration(const RuntimeProfile *p,
                                    EnvBoxDnsFieldSetter set, void *ctx) {
  const char *keys[] = {"identity_computer_name", "identity_user_name",
                        "identity_mac_address", "identity_machine_guid"};
  const wchar_t *fields[] = {p->identity_computer_name, p->identity_user_name,
                             p->identity_mac_address, p->identity_machine_guid};
  for (int i = 0; i < 4; ++i)
    if (fields[i][0]) {
      char text[65];
      size_t j = 0;
      for (; fields[i][j] && j < 64; ++j) text[j] = (char)fields[i][j];
      text[j] = 0;
      if (!set(ctx, keys[i], text)) return 0;
    }
  return 1;
}
void EnvBoxApplyIdentityEnvironment(const RuntimeProfile *p) {
  // Reserved bootstrap variables must come exclusively from this immutable
  // snapshot, including packaged roots whose inherited environment is unknown.
  wchar_t *block = GetEnvironmentStringsW();
  std::vector<std::wstring> remove;
  if (block) {
    for (const wchar_t *entry = block; *entry; entry += wcslen(entry) + 1) {
      if (_wcsnicmp(entry, L"ENVBOX_IDENTITY_", 16) != 0) continue;
      const wchar_t *eq = wcschr(entry, L'=');
      if (eq) remove.emplace_back(entry, eq - entry);
    }
    FreeEnvironmentStringsW(block);
    for (const auto &key : remove)
      SetEnvironmentVariableW(key.c_str(), nullptr);
  }
  const wchar_t *keys[] = {
      L"ENVBOX_IDENTITY_COMPUTER_NAME", L"ENVBOX_IDENTITY_USER_NAME",
      L"ENVBOX_IDENTITY_MAC_ADDRESS", L"ENVBOX_IDENTITY_MACHINE_GUID"};
  const wchar_t *fields[] = {p->identity_computer_name, p->identity_user_name,
                             p->identity_mac_address, p->identity_machine_guid};
  for (int i = 0; i < 4; ++i)
    SetEnvironmentVariableW(keys[i], fields[i][0] ? fields[i] : nullptr);
  if (p->identity_computer_name[0])
    SetEnvironmentVariableW(L"COMPUTERNAME", p->identity_computer_name);
  if (p->identity_user_name[0])
    SetEnvironmentVariableW(L"USERNAME", p->identity_user_name);
}
