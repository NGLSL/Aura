#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <psapi.h>
#include <iphlpapi.h>
#include <stdio.h>
#include <stdint.h>

struct ProbeOutcome {
  uint32_t error;
  int32_t length;
  uint16_t query_id;
  uint16_t response_id;
  uint8_t qr;
  uint8_t rcode;
  uint8_t question_match;
  uint8_t response_shape;
};

static const char* ErrorName(uint32_t error) {
  switch (error) {
    case 0: return "none";
    case 1: return "argument";
    case 2: return "cancelled";
    case 3: return "deadline";
    case 4: return "network";
    case 5: return "tls";
    case 6: return "certificate";
    case 7: return "revocation_unknown";
    case 8: return "revoked";
    case 9: return "identity";
    case 10: return "disallowed";
    case 11: return "trust_snapshot";
    case 12: return "http_status";
    case 13: return "media_type";
    case 14: return "body_limit";
    case 15: return "http";
    case 16: return "panic";
    case 17: return "content_encoding";
    default: return "unknown";
  }
}

static bool HasRuntimeModule() {
  HMODULE modules[1024] = {};
  DWORD bytes = 0;
  HANDLE process = GetCurrentProcess();
  if (!EnumProcessModules(process, modules, sizeof(modules), &bytes)) return true;
  const DWORD count = bytes / sizeof(HMODULE);
  for (DWORD i = 0; i < count; ++i) {
    wchar_t name[MAX_PATH] = {};
    if (GetModuleBaseNameW(process, modules[i], name, MAX_PATH) &&
        (wcsstr(name, L"envbox-runtime") != nullptr)) return true;
  }
  return false;
}

static bool HasIpv6Route() {
  ULONG size = 0;
  if (GetAdaptersAddresses(AF_INET6, GAA_FLAG_INCLUDE_PREFIX, nullptr, nullptr, &size) != ERROR_BUFFER_OVERFLOW) return false;
  auto* bytes = new unsigned char[size];
  auto* addresses = reinterpret_cast<IP_ADAPTER_ADDRESSES*>(bytes);
  const ULONG result = GetAdaptersAddresses(AF_INET6, GAA_FLAG_INCLUDE_PREFIX, nullptr, addresses, &size);
  bool usable = false;
  if (result == NO_ERROR) {
    for (auto* adapter = addresses; adapter; adapter = adapter->Next) {
      if (adapter->OperStatus != IfOperStatusUp) continue;
      for (auto* unicast = adapter->FirstUnicastAddress; unicast; unicast = unicast->Next) {
        if (unicast->Address.lpSockaddr && unicast->Address.lpSockaddr->sa_family == AF_INET6) {
          const auto* address = reinterpret_cast<const sockaddr_in6*>(unicast->Address.lpSockaddr);
          if (!IN6_IS_ADDR_LOOPBACK(&address->sin6_addr) && !IN6_IS_ADDR_LINKLOCAL(&address->sin6_addr)) usable = true;
        }
      }
    }
  }
  delete[] bytes;
  return usable;
}

int wmain(int argc, wchar_t** argv) {
  if (argc != 2) return 10;
  if (HasRuntimeModule()) {
    wprintf(L"probe_pid=%lu runtime_modules=1 error=host_injected\n", GetCurrentProcessId());
    return 11;
  }
  HMODULE module = LoadLibraryW(argv[1]);
  if (!module) {
    wprintf(L"probe_pid=%lu runtime_modules=0 error=load:%lu\n", GetCurrentProcessId(), GetLastError());
    return 12;
  }
  using Run = int (__cdecl*)(ProbeOutcome*);
  auto run = reinterpret_cast<Run>(GetProcAddress(module, "RunNativeDohProbe"));
  if (!run) { FreeLibrary(module); return 13; }
  ProbeOutcome outcomes[4] = {};
  const int status = run(outcomes);
  printf("probe_pid=%lu runtime_modules=0 status=%d ipv6_route=%s\n",
      GetCurrentProcessId(), status, HasIpv6Route() ? "yes" : "no");
  const char* names[] = {"cloudflare_ipv4", "cloudflare_ipv6", "google_ipv4", "google_ipv6"};
  const char* urls[] = {
      "https://cloudflare-dns.com/dns-query", "https://cloudflare-dns.com/dns-query",
      "https://dns.google/dns-query", "https://dns.google/dns-query"};
  const char* ips[] = {"1.1.1.1", "2606:4700:4700::1111", "8.8.8.8", "2001:4860:4860::8888"};
  for (int i = 0; i < 4; ++i) {
    const ProbeOutcome& out = outcomes[i];
    printf("case=%s url=%s literal_ip=%s error=%u error_name=%s length=%d query_id=%04x response_id=%04x qr=%u rcode=%u question_match=%u response_shape=%u\n",
        names[i], urls[i], ips[i], out.error, ErrorName(out.error), out.length,
        out.query_id, out.response_id, out.qr, out.rcode,
        out.question_match, out.response_shape);
  }
  FreeLibrary(module);
  // A public positive is judged by the caller from the recorded wire/error
  // fields. Keep the process successful when IPv6 is unavailable or a trust
  // policy rejects the endpoint, so the evidence preserves the reason.
  return status;
}
