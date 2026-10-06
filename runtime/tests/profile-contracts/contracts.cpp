#include "../../src/browser_child_locale.h"
#include "../../src/runtime_profile.h"
#include <cstdio>
#include <cstdlib>
#include <map>
#include <shellapi.h>
#include <string>
#include <windows.h>
using Fields = std::map<std::string, std::string>;
static void Check(bool ok, const char *message) {
  if (!ok) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}
static int Get(void *context, const char *key, char *out, size_t cap) {
  auto &fields = *static_cast<Fields *>(context);
  auto i = fields.find(key);
  if (i == fields.end()) return 0;
  if (i->second.size() >= cap) return -1;
  strcpy_s(out, cap, i->second.c_str());
  return 1;
}
static int Set(void *context, const char *key, const char *value) {
  (*static_cast<Fields *>(context))[key] = value;
  return 1;
}
int main() {
  RuntimeProfile p = {};
  Fields input, output;
  Check(EnvBoxDecodeIdentityConfiguration(&p, Get, &input) == 1,
        "missing identity is Host");
  Check(
      EnvBoxEmitIdentityConfiguration(&p, Set, &output) == 1 && output.empty(),
      "Host has no new digest tokens");
  input = {{"identity_computer_name", "Profile-Alpha"},
           {"identity_user_name", "profile.user"},
           {"identity_mac_address", "02:AA:BB:CC:DD:11"},
           {"identity_machine_guid", "12345678-1234-1234-1234-123456789abc"}};
  Check(EnvBoxDecodeIdentityConfiguration(&p, Get, &input) == 1,
        "four canonical values accepted");
  Check(p.identity_mac_bytes[0] == 2 && p.identity_mac_bytes[5] == 17,
        "MAC decoder bytes");
  Check(
      EnvBoxEmitIdentityConfiguration(&p, Set, &output) == 1 && output == input,
      "identity round trip");
  for (const auto &invalid : Fields{
           {"identity_computer_name", "too-long-hostname"},
           {"identity_user_name", "domain\\name"},
           {"identity_mac_address", "03:AA:BB:CC:DD:11"},
           {"identity_machine_guid", "12345678-1234-1234-1234-123456789ABC"}}) {
    Fields bad = input;
    bad[invalid.first] = invalid.second;
    Check(!EnvBoxDecodeIdentityConfiguration(&p, Get, &bad),
          "invalid identities rejected");
    bad[invalid.first] = "";
    Check(!EnvBoxDecodeIdentityConfiguration(&p, Get, &bad),
          "present empty identity rejected");
  }
  Fields bad = input;
  bad["identity_mac_address"] = "00:00:00:00:00:00";
  Check(!EnvBoxDecodeIdentityConfiguration(&p, Get, &bad), "zero MAC rejected");
  bad = input;
  bad["identity_machine_guid"] = "00000000-0000-0000-0000-000000000000";
  Check(!EnvBoxDecodeIdentityConfiguration(&p, Get, &bad), "nil GUID rejected");
  Check(EnvBoxDecodeIdentityConfiguration(&p, Get, &input) == 1,
        "restore canonical profile");
  SetEnvironmentVariableW(L"ENVBOX_IDENTITY_UNKNOWN", L"poison");
  SetEnvironmentVariableW(L"USERNAME", L"poison");
  EnvBoxApplyIdentityEnvironment(&p);
  wchar_t buffer[100];
  Check(GetEnvironmentVariableW(L"ENVBOX_IDENTITY_UNKNOWN", buffer, 100) == 0,
        "reserved unknown variables cleared");
  Check(GetEnvironmentVariableW(L"USERNAME", buffer, 100) > 0 &&
            wcscmp(buffer, L"profile.user") == 0,
        "USERNAME consistency");
  std::wstring cmd =
      L"\"C:\\Program Files\\Edge\\msedge.exe\" --LANG=zh-CN "
      L"--accept-lang zh-CN \"https://example.test/a b\" "
      L"--user-data-dir=\"C:\\Temp\\browser profile\\\\\"";
  Check(
      EnvBoxCommandImage(cmd.c_str()) == L"C:\\Program Files\\Edge\\msedge.exe",
      "quoted image extraction");
  Check(EnvBoxEnsureChromiumLocale(&cmd, L"ja-JP") == 1,
        "child locale rewritten");
  int count = 0;
  auto args = CommandLineToArgvW(cmd.c_str(), &count);
  Check(args && count == 5, "all other arguments retained");
  Check(wcscmp(args[1], L"--lang=ja-JP") == 0 &&
            wcscmp(args[2], L"--accept-lang=ja-JP,ja") == 0,
        "Profile flags precede URLs");
  Check(
      wcscmp(args[3], L"https://example.test/a b") == 0 &&
          wcscmp(args[4], L"--user-data-dir=C:\\Temp\\browser profile\\") == 0,
      "spaces and trailing slash preserved");
  LocalFree(args);
  Check(EnvBoxEnsureChromiumLocale(&cmd, L"ja-JP") == 0,
        "locale rewrite idempotent");
  std::wstring only =
      EnvBoxQuoteWindowsArgument(L"C:\\Program Files\\Edge\\msedge.exe");
  Check(EnvBoxEnsureChromiumLocale(&only, L"en") == 1,
        "null-command image locale");
  puts(
      "PASS profile identity decoder, reserved environment, browser child "
      "argv contracts");
}
