#include <winsock2.h>
#include <windows.h>
#include <tlhelp32.h>
#include "runtime_profile.h"
#include <cstdio>
#include <cstring>
#include <map>
#include <string>

using Fields = std::map<std::string, std::string>;

static int Get(void* context, const char* key, char* value, size_t capacity) {
  const auto& fields = *static_cast<const Fields*>(context);
  const auto found = fields.find(key);
  if (found == fields.end()) return 0;
  if (found->second.size() >= capacity) return -1;
  memcpy(value, found->second.c_str(), found->second.size() + 1);
  return 1;
}

static int Set(void* context, const char* key, const char* value) {
  (*static_cast<Fields*>(context))[key] = value;
  return 1;
}

static Fields DoH(const char* policy) {
  return {{"dns_config_version", "1"}, {"dns_mode", "virtual_view"},
      {"dns_strict", "1"}, {"dns_upstream_count", "1"},
      {"dns_upstream_0_type", "doh"},
      {"dns_upstream_0_url", "https://cloudflare-dns.com/dns-query"},
      {"dns_upstream_0_bootstrap_count", "1"},
      {"dns_upstream_0_bootstrap_0", "1.1.1.1"},
      {"dns_upstream_0_tls_revocation", policy}};
}

static bool RoundTrip(Fields fields, int expected_policy, int expected_mode, int expected_strict) {
  RuntimeProfile decoded = {}, restored = {};
  Fields emitted;
  return EnvBoxDecodeDnsConfiguration(&decoded, Get, &fields) == 1 &&
      decoded.dns_config_version == 1 && decoded.dns_mode == expected_mode &&
      decoded.dns_strict == expected_strict && decoded.dns_upstream_count == 1 &&
      decoded.dns_upstreams[0].type == EnvBoxDnsDoh &&
      decoded.dns_upstreams[0].tls_revocation == expected_policy &&
      decoded.dns_upstreams[0].bootstrap_count == 1 &&
      strcmp(decoded.dns_upstreams[0].bootstrap_ips[0], "1.1.1.1") == 0 &&
      EnvBoxEmitDnsConfiguration(&decoded, Set, &emitted) == 1 &&
      emitted == fields && EnvBoxDecodeDnsConfiguration(&restored, Get, &emitted) == 1 &&
      restored.dns_mode == expected_mode && restored.dns_strict == expected_strict &&
      restored.dns_upstreams[0].tls_revocation == expected_policy;
}

static bool Reject(Fields fields) {
  RuntimeProfile profile = {};
  return EnvBoxDecodeDnsConfiguration(&profile, Get, &fields) == 0;
}

static bool Uninjected() {
  HANDLE modules = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32,
      GetCurrentProcessId());
  if (modules == INVALID_HANDLE_VALUE) return false;
  MODULEENTRY32W entry = {};
  entry.dwSize = sizeof(entry);
  bool clean = Module32FirstW(modules, &entry) != FALSE;
  if (clean) {
    do {
      if (wcsstr(entry.szModule, L"envbox-runtime")) clean = false;
    } while (Module32NextW(modules, &entry));
  }
  CloseHandle(modules);
  return clean;
}

int main() {
  const bool clean = Uninjected();
  printf("probe_pid=%lu runtime_modules=%u\n", GetCurrentProcessId(), clean ? 0 : 1);
  if (!clean) return 12;
  int failures = 0;
  auto check = [&](const char* name, bool passed) {
    printf("case=%s result=%s\n", name, passed ? "PASS" : "FAIL");
    if (!passed) ++failures;
  };
  check("standard_doh_roundtrip", RoundTrip(DoH("0"), 0, 1, 1));
  check("strict_offline_doh_roundtrip", RoundTrip(DoH("1"), 1, 1, 1));
  Fields fields = DoH("0");
  fields.erase("dns_upstream_0_tls_revocation");
  check("missing_tls_policy_rejected", Reject(fields));
  check("unknown_tls_policy_rejected", Reject(DoH("2")));
  fields = DoH("0");
  fields["dns_strict"] = "0";
  check("virtual_view_nonstrict_rejected", Reject(fields));
  fields["dns_strict"] = "false";
  check("virtual_view_nonstrict_alias_rejected", Reject(fields));
  fields["dns_mode"] = "host";
  fields["dns_strict"] = "0";
  check("host_nonstrict_allowed", RoundTrip(fields, 0, 0, 0));
  printf("failures=%d\n", failures);
  return failures ? 1 : 0;
}
