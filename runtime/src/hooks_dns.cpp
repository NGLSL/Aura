// DNS View hooks (ticket 08) + DNS routing (ticket 25).
// DNS View: virtualize *read* of DNS config only (GetNetworkParams /
// GetAdaptersAddresses). DNS routing: under VirtualView, resolve names through
// Profile dns_servers via a bounded UDP/TCP client. DnsQuery* uses the native
// wire-record decoder for all QTYPEs and returns Profile errors without Host
// DNS fallback. The synchronous getaddrinfo routes retain their address-only
// client and return Profile lookup errors without Host fallback.
// Non-goals: WFP / LSP / port-53 redirect /
// system proxy. No new Process/Thread handles; sockets always closesocket.

#pragma comment(lib, "dnsapi.lib")

#include <winsock2.h>

#include <ws2tcpip.h>

#include <iphlpapi.h>

#include <windns.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "hooks.h"
#include "runtime_profile.h"

#include "audit.h"
#include "dns_transport.h"
#include "dns_doh.h"
#include "dns_response_cache.h"

static DWORD(WINAPI* TrueGetNetworkParams)(PFIXED_INFO, PULONG) =
    GetNetworkParams;
static ULONG(WINAPI* TrueGetAdaptersAddresses)(ULONG, ULONG, PVOID,
                                               PIP_ADAPTER_ADDRESSES, PULONG) =
    GetAdaptersAddresses;

// Resolve-path entry points (installed only when VirtualView + servers).
static INT(WSAAPI* Truegetaddrinfo)(PCSTR, PCSTR, const ADDRINFOA*,
                                    PADDRINFOA*) = getaddrinfo;
static INT(WSAAPI* TrueGetAddrInfoW)(PCWSTR, PCWSTR, const ADDRINFOW*,
                                     PADDRINFOW*) = GetAddrInfoW;
static void(WSAAPI* Truefreeaddrinfo)(PADDRINFOA) = freeaddrinfo;
static void(WSAAPI* TrueFreeAddrInfoW)(PADDRINFOW) = FreeAddrInfoW;
static INT(WSAAPI* TrueGetAddrInfoExA)(
    PCSTR, PCSTR, DWORD, LPGUID, const ADDRINFOEXA*, PADDRINFOEXA*,
    struct timeval*, LPOVERLAPPED, LPLOOKUPSERVICE_COMPLETION_ROUTINE,
    LPHANDLE) = GetAddrInfoExA;
static INT(WSAAPI* TrueGetAddrInfoExW)(
    PCWSTR, PCWSTR, DWORD, LPGUID, const ADDRINFOEXW*, PADDRINFOEXW*,
    struct timeval*, LPOVERLAPPED, LPLOOKUPSERVICE_COMPLETION_ROUTINE,
    LPHANDLE) = GetAddrInfoExW;
static INT(WSAAPI* TrueGetAddrInfoExCancel)(LPHANDLE) = GetAddrInfoExCancel;
static void(WSAAPI* TrueFreeAddrInfoExA)(PADDRINFOEXA) = FreeAddrInfoExA;
static void(WSAAPI* TrueFreeAddrInfoExW)(PADDRINFOEXW) = FreeAddrInfoExW;
static DNS_STATUS(WINAPI* TrueDnsQuery_A)(PCSTR, WORD, DWORD, PVOID,
                                          PDNS_RECORD*, PVOID*) = DnsQuery_A;
static DNS_STATUS(WINAPI* TrueDnsQuery_W)(PCWSTR, WORD, DWORD, PVOID,
                                          PDNS_RECORD*, PVOID*) = DnsQuery_W;
static DNS_STATUS(WINAPI* TrueDnsQuery_UTF8)(PCSTR, WORD, DWORD, PVOID,
                                             PDNS_RECORD*,
                                             PVOID*) = DnsQuery_UTF8;
static DNS_STATUS(WINAPI* TrueDnsQueryEx)(PDNS_QUERY_REQUEST,
                                          PDNS_QUERY_RESULT,
                                          PDNS_QUERY_CANCEL) = DnsQueryEx;
static DNS_STATUS(WINAPI* TrueDnsCancelQuery)(PDNS_QUERY_CANCEL) =
    DnsCancelQuery;
static void(WINAPI* TrueDnsFree)(PVOID, DNS_FREE_TYPE) = DnsFree;
// Dynamic import keeps Windows versions without the Raw API loadable. Both
// parameters are opaque here because strict rejects before reading them.
static DNS_STATUS(WINAPI* TrueDnsQueryRaw)(void*, void*) = nullptr;

// Process-immutable virtual DNS views (built once at hook install).
static int g_view_active = 0;
// Windows resolves localhost through a newer DnsQueryEx request internally.
// Keep this permission scoped to the synchronous local resolver invocation:
// it must never turn an unrelated nested query into a Host DNS fallback.
static thread_local unsigned int g_localhost_resolution_depth = 0;
class LocalhostResolutionScope {
 public:
  explicit LocalhostResolutionScope(const wchar_t* name)
      : active_(name && (_wcsicmp(name, L"localhost") == 0 ||
                         _wcsicmp(name, L"localhost.") == 0)) {
    if (active_) ++g_localhost_resolution_depth;
  }
  explicit LocalhostResolutionScope(const char* name)
      : active_(name && (_stricmp(name, "localhost") == 0 ||
                         _stricmp(name, "localhost.") == 0)) {
    if (active_) ++g_localhost_resolution_depth;
  }
  ~LocalhostResolutionScope() {
    if (active_) --g_localhost_resolution_depth;
  }
  LocalhostResolutionScope(const LocalhostResolutionScope&) = delete;
  LocalhostResolutionScope& operator=(const LocalhostResolutionScope&) = delete;
 private:
  bool active_;
};
// A custom cancel token is safe only when its matching cancel hook attached.
static int g_dns_cancel_hook_attached = 0;
static int g_dns_hooks_ready = 0;
static int g_net_count = 0;
static int g_addr_v4_count = 0;
static int g_addr_v6_count = 0;
static IP_ADDR_STRING g_net_nodes[ENVBOX_DNS_MAX];
static IP_ADAPTER_DNS_SERVER_ADDRESS g_addr_v4[ENVBOX_DNS_MAX];
static IP_ADAPTER_DNS_SERVER_ADDRESS g_addr_v6[ENVBOX_DNS_MAX];
static sockaddr_storage g_sa_v4[ENVBOX_DNS_MAX];
static sockaddr_storage g_sa_v6[ENVBOX_DNS_MAX];

// Bounded DNS client (never permanently block).
static const DWORD kDnsQueryTimeoutMs = 1800;
static const DWORD kDnsTotalBudgetMs = 6000;
static const int kDnsMaxAnswers = 8;
static const int kDnsMaxCnameHops = 8;

// The wire client uses Winsock directly. dnsapi/getaddrinfo usually happen to
// initialize Winsock for callers, but a process that enters a Profile-routed
// query first is allowed to have no WSAStartup reference yet. Keep one
// symmetric reference around each complete wire route so Runtime startup does
// not permanently change the host process's Winsock reference count.
static int StartDnsWinsock() {
  WSADATA data = {};
  int status = WSAStartup(MAKEWORD(2, 2), &data);
  if (status != 0) {
    SetLastError((DWORD)status);
    return 0;
  }
  if (data.wVersion != MAKEWORD(2, 2)) {
    WSACleanup();
    SetLastError(WSAVERNOTSUPPORTED);
    return 0;
  }
  return 1;
}

struct DnsWinsockScope {
  int active;

  DnsWinsockScope() : active(StartDnsWinsock()) {}

  ~DnsWinsockScope() {
    if (active) {
      WSACleanup();
    }
  }
};

// UDP port for Profile DNS. Production uses 53. ENVBOX_DNS_UDP_PORT is a
// test seam so fixtures can avoid fighting host DNS proxies on :53.
static unsigned DnsUdpPort() {
  char buf[16];
  DWORD n = GetEnvironmentVariableA("ENVBOX_DNS_UDP_PORT", buf, (DWORD)sizeof(buf));
  if (n > 0 && n < (DWORD)sizeof(buf)) {
    unsigned p = (unsigned)strtoul(buf, nullptr, 10);
    if (p > 0 && p <= 65535) {
      return p;
    }
  }
  return 53;
}

static int DnsUpstreamCount(const RuntimeProfile* profile) {
  int count = profile->dns_config_version == 1 ? profile->dns_upstream_count : profile->dns_server_count;
  return count > 0 && count <= ENVBOX_DNS_MAX ? count : 0;
}

static int DnsEndpoint(const RuntimeProfile* profile, int index, DnsTransportEndpoint* endpoint) {
  if (profile->dns_config_version == 1) {
    const RuntimeDnsUpstream& configured = profile->dns_upstreams[index];
    if (configured.type == EnvBoxDnsDot) {
      *endpoint = {DnsTransportKind::Dot, configured.address, configured.port,
                   configured.server_name};
      return 1;
    }
    if (configured.type == EnvBoxDnsDoh) {
      *endpoint = {DnsTransportKind::Doh, nullptr, 0, nullptr,
                   configured.url, configured.bootstrap_ips,
                   configured.bootstrap_count,
                   static_cast<unsigned>(configured.tls_revocation)};
      return 1;
    }
    if (configured.type != EnvBoxDnsUdp && configured.type != EnvBoxDnsTcp) return 0;
    *endpoint = {configured.type == EnvBoxDnsTcp ? DnsTransportKind::Tcp : DnsTransportKind::Udp,
                 configured.address, configured.port};
  } else {
    *endpoint = {DnsTransportKind::Udp, profile->dns_servers[index],
                 static_cast<unsigned short>(DnsUdpPort())};
  }
  return 1;
}

// Owned-allocation registry so address-info free APIs can release our nodes
// even when the CRT heap differs from ws2_32.
#ifndef ENVBOX_OWNED_MAX
// Bound custom address-info allocations held by concurrent resolver calls.
#define ENVBOX_OWNED_MAX 2048
#endif
static void* g_owned[ENVBOX_OWNED_MAX];
static SRWLOCK g_owned_lock = SRWLOCK_INIT;

// Returns 1 when tracked. Table-full must not hand out untracked pointers
// (free would then take the True* path on HeapAlloc memory).
static int OwnedAdd(void* p) {
  if (p == nullptr) {
    return 0;
  }
  int ok = 0;
  AcquireSRWLockExclusive(&g_owned_lock);
  for (int i = 0; i < ENVBOX_OWNED_MAX; i++) {
    if (g_owned[i] == nullptr) {
      g_owned[i] = p;
      ok = 1;
      break;
    }
  }
  ReleaseSRWLockExclusive(&g_owned_lock);
  return ok;
}

static int OwnedHas(void* p) {
  if (p == nullptr) {
    return 0;
  }
  int found = 0;
  AcquireSRWLockShared(&g_owned_lock);
  for (int i = 0; i < ENVBOX_OWNED_MAX; i++) {
    if (g_owned[i] == p) {
      found = 1;
      break;
    }
  }
  ReleaseSRWLockShared(&g_owned_lock);
  return found;
}

static void OwnedRemove(void* p) {
  if (p == nullptr) {
    return;
  }
  AcquireSRWLockExclusive(&g_owned_lock);
  for (int i = 0; i < ENVBOX_OWNED_MAX; i++) {
    if (g_owned[i] == p) {
      g_owned[i] = nullptr;
      break;
    }
  }
  ReleaseSRWLockExclusive(&g_owned_lock);
}

static void* OwnedAlloc(size_t n) {
  void* p = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, n);
  if (p == nullptr) {
    return nullptr;
  }
  if (!OwnedAdd(p)) {
    HeapFree(GetProcessHeap(), 0, p);
    return nullptr;
  }
  return p;
}

static void OwnedFree(void* p) {
  if (p == nullptr) {
    return;
  }
  OwnedRemove(p);
  HeapFree(GetProcessHeap(), 0, p);
}

static int IsIpv4Text(const char* text) {
  in_addr a;
  return InetPtonA(AF_INET, text, &a) == 1;
}

static void BuildDnsView() {
  const RuntimeProfile* pfl = EnvBoxProfile();
  g_view_active = 0;
  g_net_count = 0;
  g_addr_v4_count = 0;
  g_addr_v6_count = 0;
  if (pfl == nullptr || pfl->dns_mode != 1) {
    return;
  }
  g_view_active = 1;
  int n = pfl->dns_server_count > 0 && pfl->dns_server_count <= ENVBOX_DNS_MAX ? pfl->dns_server_count : 0;
  for (int i = 0; i < n; i++) {
    const char* text = pfl->dns_servers[i];

    if (IsIpv4Text(text) && g_net_count < ENVBOX_DNS_MAX) {
      int ni = g_net_count++;
      memset(&g_net_nodes[ni], 0, sizeof(g_net_nodes[ni]));
      _snprintf_s(g_net_nodes[ni].IpAddress.String,
                  sizeof(g_net_nodes[ni].IpAddress.String), _TRUNCATE, "%s",
                  text);
      g_net_nodes[ni].Next = nullptr;
      if (ni > 0) {
        g_net_nodes[ni - 1].Next = &g_net_nodes[ni];
      }

      int ai = g_addr_v4_count++;
      memset(&g_sa_v4[ai], 0, sizeof(g_sa_v4[ai]));
      sockaddr_in v4;
      memset(&v4, 0, sizeof(v4));
      v4.sin_family = AF_INET;
      InetPtonA(AF_INET, text, &v4.sin_addr);
      memcpy(&g_sa_v4[ai], &v4, sizeof(v4));
      memset(&g_addr_v4[ai], 0, sizeof(g_addr_v4[ai]));
      g_addr_v4[ai].Length = sizeof(IP_ADAPTER_DNS_SERVER_ADDRESS);
      g_addr_v4[ai].Address.lpSockaddr =
          reinterpret_cast<LPSOCKADDR>(&g_sa_v4[ai]);
      g_addr_v4[ai].Address.iSockaddrLength = (int)sizeof(sockaddr_in);
      g_addr_v4[ai].Next = nullptr;
      if (ai > 0) {
        g_addr_v4[ai - 1].Next = &g_addr_v4[ai];
      }
      continue;
    }

    in6_addr a6;
    memset(&a6, 0, sizeof(a6));
    if (InetPtonA(AF_INET6, text, &a6) == 1 && g_addr_v6_count < ENVBOX_DNS_MAX) {
      int ai = g_addr_v6_count++;
      memset(&g_sa_v6[ai], 0, sizeof(g_sa_v6[ai]));
      sockaddr_in6 v6;
      memset(&v6, 0, sizeof(v6));
      v6.sin6_family = AF_INET6;
      v6.sin6_addr = a6;
      memcpy(&g_sa_v6[ai], &v6, sizeof(v6));
      memset(&g_addr_v6[ai], 0, sizeof(g_addr_v6[ai]));
      g_addr_v6[ai].Length = sizeof(IP_ADAPTER_DNS_SERVER_ADDRESS);
      g_addr_v6[ai].Address.lpSockaddr =
          reinterpret_cast<LPSOCKADDR>(&g_sa_v6[ai]);
      g_addr_v6[ai].Address.iSockaddrLength = (int)sizeof(sockaddr_in6);
      g_addr_v6[ai].Next = nullptr;
      if (ai > 0) {
        g_addr_v6[ai - 1].Next = &g_addr_v6[ai];
      }
    }
  }

  g_view_active = 1;
}

static DWORD WINAPI HookGetNetworkParams(PFIXED_INFO pFixedInfo,
                                         PULONG pOutBufLen) {
  DWORD status = TrueGetNetworkParams(pFixedInfo, pOutBufLen);
  if (status != ERROR_SUCCESS || pFixedInfo == nullptr) {
    EnvBoxAuditEvent("GetNetworkParams", 0, "fail-open");
    return status;
  }
  if (!g_view_active) {
    EnvBoxAuditEvent("GetNetworkParams", 0, "dns-host");
    return status;
  }
  EnvBoxAuditEvent("GetNetworkParams", 1, "dns-virtual-view");
  // Always replace (never leave Host list when VirtualView is active).
  // CurrentDnsServer points at the first Profile server or is cleared.
  pFixedInfo->CurrentDnsServer = nullptr;
  memset(&pFixedInfo->DnsServerList, 0, sizeof(pFixedInfo->DnsServerList));
  if (g_net_count > 0) {
    pFixedInfo->DnsServerList = g_net_nodes[0];
    pFixedInfo->CurrentDnsServer = &g_net_nodes[0];
  }
  return status;
}

static IP_ADAPTER_DNS_SERVER_ADDRESS* ChainForFamily(ULONG Family,
                                                     int* out_count) {
  // AF_UNSPEC=0, AF_INET=2, AF_INET6=23 (Winsock).
  if (Family == AF_INET) {
    *out_count = g_addr_v4_count;
    return g_addr_v4_count > 0 ? &g_addr_v4[0] : nullptr;
  }
  if (Family == AF_INET6) {
    *out_count = g_addr_v6_count;
    return g_addr_v6_count > 0 ? &g_addr_v6[0] : nullptr;
  }
  // AF_UNSPEC / other: IPv4 chain then IPv6 chain.
  if (g_addr_v4_count > 0 && g_addr_v6_count > 0) {
    g_addr_v4[g_addr_v4_count - 1].Next = &g_addr_v6[0];
    *out_count = g_addr_v4_count + g_addr_v6_count;
    return &g_addr_v4[0];
  }
  if (g_addr_v4_count > 0) {
    *out_count = g_addr_v4_count;
    return &g_addr_v4[0];
  }
  *out_count = g_addr_v6_count;
  return g_addr_v6_count > 0 ? &g_addr_v6[0] : nullptr;
}

static ULONG WINAPI HookGetAdaptersAddresses(
    ULONG Family, ULONG Flags, PVOID Reserved,
    PIP_ADAPTER_ADDRESSES AdapterAddresses, PULONG SizePointer) {
  ULONG status = TrueGetAdaptersAddresses(Family, Flags, Reserved,
                                          AdapterAddresses, SizePointer);
  if (status != ERROR_SUCCESS || AdapterAddresses == nullptr) {
    EnvBoxAuditEvent("GetAdaptersAddresses", 0, "fail-open");
    return status;
  }
  const RuntimeProfile* identity = EnvBoxProfile();
  if (identity->identity_mac_address[0]) {
    for (auto* a = AdapterAddresses; a != nullptr; a = a->Next) {
      if (a->Length >= offsetof(IP_ADAPTER_ADDRESSES, PhysicalAddressLength) + sizeof(a->PhysicalAddressLength) && a->PhysicalAddressLength == 6)
        memcpy(a->PhysicalAddress, identity->identity_mac_bytes, 6);
    }
  }
  if (!g_view_active) {
    EnvBoxAuditEvent("GetAdaptersAddresses", 0, "dns-host");
    return status;
  }
  EnvBoxAuditEvent("GetAdaptersAddresses", 1, "dns-virtual-view");
  int count = 0;
  IP_ADAPTER_DNS_SERVER_ADDRESS* chain = ChainForFamily(Family, &count);
  for (PIP_ADAPTER_ADDRESSES a = AdapterAddresses; a != nullptr; a = a->Next) {
    // Empty Profile list for this family still replaces Host (consistent).
    a->FirstDnsServerAddress = chain;
  }
  return status;
}

// ---------------------------------------------------------------------------
// Minimal DNS wire client (ticket 25). UDP/53 only. A (1) and AAAA (28).
// ---------------------------------------------------------------------------

struct DnsAddrs {
  int n_v4;
  int n_v6;
  in_addr v4[kDnsMaxAnswers];
  in6_addr v6[kDnsMaxAnswers];
  int got;        // any DNS response received
  int nxdomain;   // rcode == 3
  int noerror;    // rcode == 0
  int nodata;     // NOERROR + authoritative SOA, with no A/AAAA/CNAME
  int has_cname;  // answer included a CNAME (no A/AAAA yet)
  char cname[256];
};

static void DnsAddrsClear(DnsAddrs* out) {
  memset(out, 0, sizeof(*out));
}

// Short-lived positive cache: Chrome re-resolves the same hosts constantly.
// Key is (qname, want_a, want_aaaa). Only definitive DnsRouteName answers.
#ifndef ENVBOX_DNS_CACHE_MAX
#define ENVBOX_DNS_CACHE_MAX 64
#endif
struct DnsCacheEnt {
  char name[256];
  int want_a;
  int want_aaaa;
  ULONGLONG expire;
  DnsAddrs addrs;
};
static DnsCacheEnt g_dns_cache[ENVBOX_DNS_CACHE_MAX];
static SRWLOCK g_dns_cache_lock = SRWLOCK_INIT;
static const DWORD kDnsCacheTtlMs = 30000;

static int DnsCacheLookup(const char* qname, int want_a, int want_aaaa,
                          DnsAddrs* out) {
  ULONGLONG now = GetTickCount64();
  int hit = 0;
  AcquireSRWLockShared(&g_dns_cache_lock);
  for (int i = 0; i < ENVBOX_DNS_CACHE_MAX; i++) {
    const DnsCacheEnt* e = &g_dns_cache[i];
    if (e->name[0] == '\0' || e->expire <= now) {
      continue;
    }
    if (e->want_a == want_a && e->want_aaaa == want_aaaa &&
        _stricmp(e->name, qname) == 0) {
      *out = e->addrs;
      hit = 1;
      break;
    }
  }
  ReleaseSRWLockShared(&g_dns_cache_lock);
  return hit;
}

static void DnsCacheStore(const char* qname, int want_a, int want_aaaa,
                          const DnsAddrs* addrs) {
  if (qname == nullptr || qname[0] == '\0' || strlen(qname) >= 256) {
    return;
  }
  ULONGLONG now = GetTickCount64();
  AcquireSRWLockExclusive(&g_dns_cache_lock);
  int slot = -1;
  ULONGLONG oldest = ~0ULL;
  int oldest_i = 0;
  for (int i = 0; i < ENVBOX_DNS_CACHE_MAX; i++) {
    DnsCacheEnt* e = &g_dns_cache[i];
    if (e->name[0] == '\0' || e->expire <= now) {
      if (slot < 0) {
        slot = i;
      }
      continue;
    }
    if (e->want_a == want_a && e->want_aaaa == want_aaaa &&
        _stricmp(e->name, qname) == 0) {
      slot = i;
      break;
    }
    if (e->expire < oldest) {
      oldest = e->expire;
      oldest_i = i;
    }
  }
  if (slot < 0) {
    slot = oldest_i;
  }
  DnsCacheEnt* e = &g_dns_cache[slot];
  strncpy_s(e->name, qname, _TRUNCATE);
  e->want_a = want_a;
  e->want_aaaa = want_aaaa;
  e->expire = now + kDnsCacheTtlMs;
  e->addrs = *addrs;
  ReleaseSRWLockExclusive(&g_dns_cache_lock);
}

static void WriteU16(unsigned char* p, unsigned v) {
  p[0] = (unsigned char)((v >> 8) & 0xff);
  p[1] = (unsigned char)(v & 0xff);
}

static void WriteU32(unsigned char* p, unsigned long v) {
  p[0] = (unsigned char)((v >> 24) & 0xff);
  p[1] = (unsigned char)((v >> 16) & 0xff);
  p[2] = (unsigned char)((v >> 8) & 0xff);
  p[3] = (unsigned char)(v & 0xff);
}

static unsigned ReadU16(const unsigned char* p) {
  return ((unsigned)p[0] << 8) | (unsigned)p[1];
}

static unsigned long ReadU32(const unsigned char* p) {
  return ((unsigned long)p[0] << 24) | ((unsigned long)p[1] << 16) |
         ((unsigned long)p[2] << 8) | (unsigned long)p[3];
}

// Encode DNS wire QNAME. Returns encoded length or 0 on error.
static int EncodeDnsName(const char* name, unsigned char* out, int cap) {
  if (name == nullptr || name[0] == '\0') return 0;
  if (strcmp(name, ".") == 0 && cap > 0) { out[0] = 0; return 1; }
  int o = 0;
  const char* p = name;
  while (*p) {
    const char* dot = strchr(p, '.');
    int lab = dot ? (int)(dot - p) : (int)strlen(p);
    if (lab <= 0 || lab > 63) {
      return 0;
    }
    if (o + 1 + lab + 1 > cap) {
      return 0;
    }
    out[o++] = (unsigned char)lab;
    memcpy(out + o, p, (size_t)lab);
    o += lab;
    if (dot == nullptr) {
      break;
    }
    p = dot + 1;
    if (*p == '\0') {
      break;  // trailing dot
    }
  }
  if (o + 1 > cap) {
    return 0;
  }
  out[o++] = 0;
  return o;
}

// Skip a (possibly compressed) DNS name at *off. Returns 1 on success.
static int SkipDnsName(const unsigned char* buf, int len, int* off) {
  int guard = 0;
  while (*off < len) {
    unsigned char c = buf[*off];
    if (c == 0) {
      (*off)++;
      return 1;
    }
    if ((c & 0xC0) == 0xC0) {
      if (*off + 2 > len) {
        return 0;
      }
      *off += 2;
      return 1;
    }
    if ((c & 0xC0) != 0) {
      return 0;
    }
    *off += 1 + (int)c;
    if (++guard > 128) {
      return 0;
    }
  }
  return 0;
}

// Decode a (possibly compressed) DNS name into presentation form.
// Advances *off past the name in the original stream (not the pointer target).
// Returns 1 on success.
static int DecodeDnsName(const unsigned char* buf, int len, int* off,
                         char* out, int cap) {
  if (off == nullptr || out == nullptr || cap <= 1 || *off < 0 || *off >= len) {
    return 0;
  }
  int cur = *off;
  int end = *off;
  int jumped = 0;
  int o = 0;
  int guard = 0;
  out[0] = '\0';
  while (guard++ < 128) {
    if (cur < 0 || cur >= len) {
      return 0;
    }
    unsigned char c = buf[cur];
    if (c == 0) {
      if (!jumped) {
        end = cur + 1;
      }
      if (o == 0) {
        out[0] = '.';
        out[1] = '\0';
      } else {
        out[o] = '\0';
      }
      *off = end;
      return 1;
    }
    if ((c & 0xC0) == 0xC0) {
      if (cur + 2 > len) {
        return 0;
      }
      int ptr = ((int)(c & 0x3F) << 8) | (int)buf[cur + 1];
      if (!jumped) {
        end = cur + 2;
        jumped = 1;
      }
      cur = ptr;
      continue;
    }
    if ((c & 0xC0) != 0) {
      return 0;
    }
    cur++;
    if (cur + (int)c > len) {
      return 0;
    }
    if (o > 0) {
      if (o + 1 >= cap) {
        return 0;
      }
      out[o++] = '.';
    }
    if (o + (int)c >= cap) {
      return 0;
    }
    memcpy(out + o, buf + cur, (size_t)c);
    o += (int)c;
    cur += (int)c;
    if (!jumped) {
      end = cur;
    }
  }
  return 0;
}

// Every API validates the same wire identity and question before decoding.
static int DnsResponseMatches(const unsigned char* packet, int length,
                               unsigned id, const char* qname, unsigned qtype) {
  if (length < 12 || ReadU16(packet) != (id & 0xffff) ||
      (ReadU16(packet + 2) & 0xf800) != 0x8000 || ReadU16(packet + 4) != 1) return 0;
  int offset = 12;
  char name[256];
  if (!DecodeDnsName(packet, length, &offset, name, sizeof(name)) ||
      offset + 4 > length || ReadU16(packet + offset) != qtype ||
      ReadU16(packet + offset + 2) != 1) return 0;
  char expected[256];
  strcpy_s(expected, qname);
  size_t n = strlen(expected);
  if (n > 1 && expected[n - 1] == '.') expected[n - 1] = '\0';
  return _stricmp(expected, name) == 0;
}

static ULONGLONG DnsAttemptDeadline(ULONGLONG total) {
  ULONGLONG bounded = GetTickCount64() + kDnsQueryTimeoutMs;
  return bounded < total ? bounded : total;
}

static int DnsBootstrapFromProfile(const char* name, char (&addresses)[ENVBOX_DNS_MAX][64],
                                  int* count, ULONGLONG deadline, HANDLE cancel_event);
static int IsAsciiNameA(const char* s);

// Downlevel dnsapi silently discards some flat records, including 64/65 and
// unknown future types. Native copying allocates these records on dnsapi's heap,
// so charset conversion and public DnsFree retain their normal ownership contract.
static decltype(&DnsExtractRecordsFromMessage_W) g_extract_dns_records =
    DnsExtractRecordsFromMessage_W;
static int DnsNameEqualsW(const wchar_t* query, const wchar_t* local);
static constexpr WORD kDnsOpaqueCarrierType = 0xff00;
struct DnsOpaqueWireRecord {
  int owner_offset;
  int data_offset;
  WORD length;
  WORD type;
  WORD section;
  DWORD ttl;
  bool present;
};
static bool IsDnsFlatWireType(WORD type) {
  // Types parsed by the default Windows DNS API. Everything else is opaque
  // wire RDATA, including NULL and future RR types (no PARSE_ALL_RECORDS option).
  switch (type) {
    case 1: case 2: case 3: case 4: case 5: case 6: case 7: case 8: case 9:
    case 11: case 12: case 13: case 14: case 15: case 16: case 17: case 18:
    case 19: case 20: case 21: case 24: case 25: case 28: case 33: case 34:
    case 35: case 39: case 41: case 43: case 46: case 47: case 48: case 49:
    case 50: case 51: case 52: case 249: case 250: case 65281: case 65282:
      return false;
    default: return true;
  }
}
struct DnsOpaqueExtractionBuffers {
  unsigned char* packet = nullptr;
  DnsOpaqueWireRecord* mapping = nullptr;
  ~DnsOpaqueExtractionBuffers() {
    if (packet) HeapFree(GetProcessHeap(), 0, packet);
    if (mapping) HeapFree(GetProcessHeap(), 0, mapping);
  }
};
static DNS_STATUS ScanDnsOpaqueWireRecords(const unsigned char* packet, int length,
                                           DnsOpaqueWireRecord* mapping,
                                           unsigned* mapped, bool* has_service_binding) {
  if (!packet || length < 12 || length > 65535) return DNS_ERROR_BAD_PACKET;
  const unsigned total = ReadU16(packet + 6) + ReadU16(packet + 8) + ReadU16(packet + 10);
  if (total > static_cast<unsigned>(length / 11)) return DNS_ERROR_BAD_PACKET;
  int offset = 12;
  for (unsigned i = 0; i < ReadU16(packet + 4); ++i) {
    if (!SkipDnsName(packet, length, &offset) || offset > length - 4)
      return DNS_ERROR_BAD_PACKET;
    offset += 4;
  }
  *mapped = 0;
  *has_service_binding = false;
  for (WORD section = 1; section <= 3; ++section) {
    const unsigned count = ReadU16(packet + 4 + section * 2);
    for (unsigned i = 0; i < count; ++i) {
      const int owner_offset = offset;
      if (!SkipDnsName(packet, length, &offset) || offset > length - 10)
        return DNS_ERROR_BAD_PACKET;
      const WORD type = ReadU16(packet + offset);
      const WORD data_length = ReadU16(packet + offset + 8);
      const DWORD ttl = ReadU32(packet + offset + 4);
      offset += 10;
      if (data_length > length - offset) return DNS_ERROR_BAD_PACKET;
      if ((type == DNS_TYPE_A && data_length != 4) ||
          (type == DNS_TYPE_AAAA && data_length != 16)) return DNS_ERROR_BAD_PACKET;
      if (IsDnsFlatWireType(type)) {
        char owner[256] = {};
        int decoded_offset = owner_offset;
        if (!DecodeDnsName(packet, length, &decoded_offset, owner, sizeof(owner)))
          return DNS_ERROR_BAD_PACKET;
        if (mapping) mapping[*mapped] = {owner_offset, offset, data_length, type, section, ttl, false};
        ++*mapped;
        *has_service_binding = true;
      }
      offset += data_length;
    }
  }
  return ERROR_SUCCESS;
}
static DNS_STATUS ExtractProfileDnsRecords(unsigned char* packet, int length,
                                           PDNS_RECORD* records) {
  if (!records) return DNS_ERROR_BAD_PACKET;
  *records = nullptr;
  unsigned mapped = 0;
  bool has_service_binding = false;
  DNS_STATUS scanned = ScanDnsOpaqueWireRecords(packet, length, nullptr, &mapped,
                                               &has_service_binding);
  if (scanned != ERROR_SUCCESS) return scanned;
  if (!has_service_binding) {
    DNS_BYTE_FLIP_HEADER_COUNTS(&reinterpret_cast<PDNS_MESSAGE_BUFFER>(packet)->MessageHead);
    DNS_STATUS status = g_extract_dns_records(reinterpret_cast<PDNS_MESSAGE_BUFFER>(packet),
                                               static_cast<WORD>(length), records);
    DNS_BYTE_FLIP_HEADER_COUNTS(&reinterpret_cast<PDNS_MESSAGE_BUFFER>(packet)->MessageHead);
    return status;
  }
  DnsOpaqueExtractionBuffers buffers;
  buffers.mapping = static_cast<DnsOpaqueWireRecord*>(HeapAlloc(
      GetProcessHeap(), 0, mapped * sizeof(DnsOpaqueWireRecord)));
  if (!buffers.mapping) return ERROR_NOT_ENOUGH_MEMORY;
  scanned = ScanDnsOpaqueWireRecords(packet, length, buffers.mapping, &mapped,
                                    &has_service_binding);
  if (scanned != ERROR_SUCCESS) return scanned;
  buffers.packet = static_cast<unsigned char*>(HeapAlloc(GetProcessHeap(), 0, length));
  if (!buffers.packet) return ERROR_NOT_ENOUGH_MEMORY;
  memcpy(buffers.packet, packet, length);
  DNS_BYTE_FLIP_HEADER_COUNTS(&reinterpret_cast<PDNS_MESSAGE_BUFFER>(buffers.packet)->MessageHead);
  DNS_STATUS status = g_extract_dns_records(
      reinterpret_cast<PDNS_MESSAGE_BUFFER>(buffers.packet), static_cast<WORD>(length), records);
  if (status == ERROR_SUCCESS || status == DNS_INFO_NO_RECORDS) {
    // Match every native flat record once, including duplicate owner/type pairs.
    for (PDNS_RECORD cursor = *records; cursor; cursor = cursor->pNext) {
      for (unsigned i = 0; i < mapped; ++i) {
        auto& source = buffers.mapping[i];
        if (source.present || cursor->wType != source.type ||
            cursor->Flags.S.Section != source.section || cursor->dwTtl != source.ttl ||
            cursor->wDataLength != source.length ||
            memcmp(&cursor->Data, packet + source.data_offset, source.length)) continue;
        char owner[256] = {};
        wchar_t owner_w[256] = {};
        int owner_offset = source.owner_offset;
        if (!DecodeDnsName(packet, length, &owner_offset, owner, sizeof(owner)) ||
            !MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, owner, -1, owner_w, ARRAYSIZE(owner_w))) {
          status = DNS_ERROR_BAD_PACKET;
          break;
        }
        if (DnsNameEqualsW(reinterpret_cast<const wchar_t*>(cursor->pName), owner_w)) {
          source.present = true;
          break;
        }
      }
      if (status == DNS_ERROR_BAD_PACKET) break;
    }
    for (unsigned i = 0; i < mapped &&
         (status == ERROR_SUCCESS || status == DNS_INFO_NO_RECORDS); ++i) {
      const auto& source = buffers.mapping[i];
      if (source.present) continue;
      char owner[256] = {};
      wchar_t owner_w[256] = {};
      int owner_offset = source.owner_offset;
      if (!DecodeDnsName(packet, length, &owner_offset, owner, sizeof(owner)) ||
          !MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, owner, -1, owner_w, ARRAYSIZE(owner_w))) {
        status = DNS_ERROR_BAD_PACKET;
        break;
      }
      const size_t allocation = FIELD_OFFSET(DNS_RECORD, Data) +
          (source.length > sizeof(DNS_RECORD::Data) ? source.length : sizeof(DNS_RECORD::Data));
      PDNS_RECORD temporary = static_cast<PDNS_RECORD>(HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, allocation));
      if (!temporary) { status = ERROR_NOT_ENOUGH_MEMORY; break; }
      temporary->pName = reinterpret_cast<decltype(temporary->pName)>(owner_w);
      temporary->wType = source.type;
      temporary->wDataLength = source.length;
      temporary->Flags.S.Section = source.section;
      temporary->Flags.S.CharSet = DnsCharSetUnicode;
      temporary->dwTtl = source.ttl;
      memcpy(&temporary->Data, packet + source.data_offset, source.length);
      PDNS_RECORD copied = DnsRecordCopyEx(temporary, DnsCharSetUnicode, DnsCharSetUnicode);
      HeapFree(GetProcessHeap(), 0, temporary);
      if (!copied) { status = ERROR_NOT_ENOUGH_MEMORY; break; }
      // Keep native parsed ordering (notably CNAME-first), and section ordering.
      PDNS_RECORD* insertion = records;
      while (*insertion && (*insertion)->Flags.S.Section <= source.section)
        insertion = &(*insertion)->pNext;
      copied->pNext = *insertion;
      *insertion = copied;
      status = ERROR_SUCCESS;
    }
  }
  if (status != ERROR_SUCCESS && *records) {
    TrueDnsFree(*records, DnsFreeRecordList);
    *records = nullptr;
  }
  return status;
}

// A Runtime Profile is immutable and this cache is local to its process. The
// endpoint's TLS policy and configured addresses still belong in the key so a
// response cannot be reused for a different upstream or transport requirement.
static std::string DnsResponseCacheKey(const DnsTransportEndpoint& endpoint,
                                       const char* name, unsigned type, DWORD options) noexcept {
  try {
    std::string key;
    auto field = [&key](const char* value) {
      key.append(value ? value : "");
      key.push_back('\0');
    };
    const RuntimeProfile* profile = EnvBoxProfile();
    if (profile) {
      key.append(reinterpret_cast<const char*>(profile->profile_id),
                 wcslen(profile->profile_id) * sizeof(wchar_t));
    }
    key.push_back('\0');
    field(endpoint.address);
    field(endpoint.server_name);
    field(endpoint.url);
    for (int i = 0; i < endpoint.bootstrap_count; ++i) field(endpoint.bootstrap_ips[i]);
    const unsigned identity[] = {static_cast<unsigned>(endpoint.kind), endpoint.port,
        endpoint.tls_revocation, static_cast<unsigned>(endpoint.bootstrap_count), type, options};
    key.append(reinterpret_cast<const char*>(identity), sizeof(identity));
    size_t count = strlen(name);
    if (count > 1 && name[count - 1] == '.') --count;
    for (size_t i = 0; i < count; ++i) {
      unsigned char c = static_cast<unsigned char>(name[i]);
      key.push_back(static_cast<char>(c >= 'A' && c <= 'Z' ? c + ('a' - 'A') : c));
    }
    return key;
  } catch (...) { return {}; }
}

// Merge simultaneous misses for the same cache key. No network work runs under
// this lock, and waiters retain their own original deadline and cancellation.
struct DnsQueryFlightSlot { std::string key; bool active = false; };
static DnsQueryFlightSlot g_dns_flights[32];
static SRWLOCK g_dns_flight_lock = SRWLOCK_INIT;
static CONDITION_VARIABLE g_dns_flight_changed = CONDITION_VARIABLE_INIT;
struct DnsQueryFlight {
  int slot = -1;
  ~DnsQueryFlight() {
    if (slot < 0) return;
    AcquireSRWLockExclusive(&g_dns_flight_lock);
    g_dns_flights[slot].active = false;
    g_dns_flights[slot].key.clear();
    WakeAllConditionVariable(&g_dns_flight_changed);
    ReleaseSRWLockExclusive(&g_dns_flight_lock);
  }
  int Enter(const std::string& key, ULONGLONG deadline, HANDLE cancel) noexcept {
    AcquireSRWLockExclusive(&g_dns_flight_lock);
    int result = 1;
    try {
      for (;;) {
        if (cancel && WaitForSingleObject(cancel, 0) == WAIT_OBJECT_0) { result = -1; break; }
        const ULONGLONG now = GetTickCount64();
        if (now >= deadline) { result = 0; break; }
        int free_slot = -1;
        bool pending = false;
        for (int i = 0; i < ARRAYSIZE(g_dns_flights); ++i) {
          if (!g_dns_flights[i].active) free_slot = i;
          else if (g_dns_flights[i].key == key) pending = true;
        }
        if (!pending) {
          if (free_slot >= 0) {
            g_dns_flights[free_slot].key = key;
            g_dns_flights[free_slot].active = true;
            slot = free_slot;
          }
          break; // A full table simply disables merging for this query.
        }
        const DWORD wait = static_cast<DWORD>((deadline - now) < 25 ? deadline - now : 25);
        if (!SleepConditionVariableSRW(&g_dns_flight_changed, &g_dns_flight_lock, wait, 0) &&
            GetLastError() != ERROR_TIMEOUT) break;
      }
    } catch (...) { /* Allocation failure disables merging, never DNS routing. */ }
    ReleaseSRWLockExclusive(&g_dns_flight_lock);
    return result;
  }
};

static int DnsQueryOne(const DnsTransportEndpoint& configured, const char* qname,
                       unsigned qtype, ULONGLONG deadline, HANDLE cancel_event,
                       DnsAddrs* out, PDNS_RECORD* records = nullptr,
                       DNS_STATUS* record_status = nullptr, DWORD options = 0) {
  if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) return -1;
  if (GetTickCount64() >= deadline) return 0;
  unsigned char qbuf[512];
  int namelen = EncodeDnsName(qname, qbuf + 12, (int)sizeof(qbuf) - 12);
  if (namelen <= 0) {
    return 0;
  }

  static volatile LONG s_qid = 0;
  unsigned id = (unsigned)(GetCurrentProcessId() + InterlockedIncrement(&s_qid));
  memset(qbuf, 0, 12);
  WriteU16(qbuf + 0, id & 0xFFFF);
  WriteU16(qbuf + 2, options & DNS_QUERY_NO_RECURSION ? 0 : 0x0100);
  WriteU16(qbuf + 4, 1);       // QDCOUNT
  WriteU16(qbuf + 6, 0);
  WriteU16(qbuf + 8, 0);
  WriteU16(qbuf + 10, 0);
  int qoff = 12 + namelen;
  WriteU16(qbuf + qoff, qtype);
  WriteU16(qbuf + qoff + 2, 1);  // IN
  qoff += 4;

  DnsTransportEndpoint endpoint = configured;
  if ((options & DNS_QUERY_USE_TCP_ONLY) && endpoint.kind == DnsTransportKind::Udp)
    endpoint.kind = DnsTransportKind::Tcp;
  const bool cache_allowed = !(options & (DNS_QUERY_BYPASS_CACHE | DNS_QUERY_WIRE_ONLY |
                                          DNS_QUERY_DONT_RESET_TTL_VALUES));
  const std::string cache_key = cache_allowed ? DnsResponseCacheKey(endpoint, qname, qtype, options)
                                             : std::string{};
  unsigned char rbuf[65535];
  int rlen = cache_key.empty() ? 0 : EnvBoxDnsResponseCache::Lookup(
      cache_key, rbuf, sizeof(rbuf), GetTickCount64());
  DnsQueryFlight flight;
  if (!rlen && !cache_key.empty()) {
    const int entered = flight.Enter(cache_key, deadline, cancel_event);
    if (entered <= 0) return entered;
    rlen = EnvBoxDnsResponseCache::Lookup(cache_key, rbuf, sizeof(rbuf), GetTickCount64());
  }
  const bool cache_hit = rlen > 0;
  if (cache_hit) WriteU16(rbuf, id & 0xffff);
  char bootstrap_name[256] = {};
  char bootstrap_addresses[ENVBOX_DNS_MAX][64] = {};
  if (!cache_hit && DnsDohBootstrapName(endpoint, bootstrap_name)) {
    int count = 0;
    int bootstrap = DnsBootstrapFromProfile(bootstrap_name, bootstrap_addresses, &count,
                                           deadline, cancel_event);
    if (bootstrap <= 0) {
      EnvBoxAuditEvent("DnsTransport.DoH", 1,
                       bootstrap < 0 ? "doh-bootstrap-cancelled" : "doh-profile-bootstrap-failed");
      return bootstrap;
    }
    endpoint.bootstrap_ips = bootstrap_addresses;
    endpoint.bootstrap_count = count;
  }
  if (!cache_hit)
    rlen = DnsTransportExchange(endpoint, qbuf, qoff, rbuf, sizeof(rbuf), deadline, cancel_event);
  if (rlen < 0) return -1;
  if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) return -1;
  if (GetTickCount64() >= deadline) return 0;
  if (!DnsResponseMatches(rbuf, rlen, id, qname, qtype)) return 0;
  if (endpoint.kind == DnsTransportKind::Udp && (ReadU16(rbuf + 2) & 0x0200)) {
    endpoint.kind = DnsTransportKind::Tcp;
    rlen = DnsTransportExchange(endpoint, qbuf, qoff, rbuf, sizeof(rbuf), deadline, cancel_event);
    if (rlen < 0) return -1;
    if (!DnsResponseMatches(rbuf, rlen, id, qname, qtype)) return 0;
  }
  unsigned flags = ReadU16(rbuf + 2);
  if ((flags & 0x8000) == 0) {
    return 0;  // not a response
  }
  // TC (0x0200): truncated answer is unusable. Never treat as final.
  if ((flags & 0x0200) != 0) {
    return 0;
  }
  unsigned rcode = flags & 0x000F;
  if (records != nullptr) {
    if (rcode != 0) {
      *record_status = DNS_ERROR_RCODE_FORMAT_ERROR + rcode - 1;
      return 1;
    }
    *record_status = ExtractProfileDnsRecords(rbuf, rlen, records);
    if (*record_status == ERROR_SUCCESS && *records == nullptr)
      *record_status = DNS_INFO_NO_RECORDS;
    if (*record_status == ERROR_SUCCESS && !cache_hit && !cache_key.empty())
      EnvBoxDnsResponseCache::Store(cache_key, rbuf, rlen, GetTickCount64());
    return 1;
  }
  unsigned qd = ReadU16(rbuf + 4);
  unsigned an = ReadU16(rbuf + 6);
  unsigned ns = ReadU16(rbuf + 8);
  int off = 12;
  for (unsigned i = 0; i < qd; i++) {
    if (!SkipDnsName(rbuf, rlen, &off)) {
      return 0;
    }
    off += 4;
    if (off > rlen) {
      return 0;
    }
  }

  out->got = 1;
  if (rcode == 3) {
    out->nxdomain = 1;
    return 1;
  }
  if (rcode == 0) {
    out->noerror = 1;
  } else {
    // SERVFAIL / REFUSED etc. are not definitive for the name.
    return 1;
  }

  for (unsigned i = 0; i < an; i++) {
    if (!SkipDnsName(rbuf, rlen, &off)) {
      return 0;
    }
    if (off + 10 > rlen) {
      return 0;
    }
    unsigned typ = ReadU16(rbuf + off);
    unsigned cls = ReadU16(rbuf + off + 2);
    unsigned rdlen = ReadU16(rbuf + off + 8);
    off += 10;
    if (cls != 1 || off + (int)rdlen > rlen) {
      return 0;
    }
    const unsigned char* rdata = rbuf + off;
    if (typ == 1 && rdlen == 4 && out->n_v4 < kDnsMaxAnswers) {
      memcpy(&out->v4[out->n_v4], rdata, 4);
      out->n_v4++;
    } else if (typ == 28 && rdlen == 16 && out->n_v6 < kDnsMaxAnswers) {
      memcpy(&out->v6[out->n_v6], rdata, 16);
      out->n_v6++;
    } else if (typ == 5 && !out->has_cname) {
      // CNAME target (may use compression into the message).
      int noff = off;
      char target[256];
      if (DecodeDnsName(rbuf, rlen, &noff, target, (int)sizeof(target)) &&
          target[0] != '\0') {
        size_t tn = strlen(target);
        if (tn > 0 && tn < sizeof(out->cname)) {
          memcpy(out->cname, target, tn + 1);
          out->has_cname = 1;
        }
      }
    }
    // Other types (NS/...) are skipped.
    off += (int)rdlen;
  }

  // RFC 2308 NODATA is a successful response whose authority section carries
  // an SOA while the answer section has no address or CNAME. An NS-only
  // authority section is a referral, so leave it to the original resolver via
  // the route's temporary-resolution-failure result.
  int has_soa = 0;
  for (unsigned i = 0; i < ns; i++) {
    if (!SkipDnsName(rbuf, rlen, &off)) {
      return 0;
    }
    if (off + 10 > rlen) {
      return 0;
    }
    unsigned typ = ReadU16(rbuf + off);
    unsigned cls = ReadU16(rbuf + off + 2);
    unsigned rdlen = ReadU16(rbuf + off + 8);
    off += 10;
    if (off + (int)rdlen > rlen) {
      return 0;
    }
    if (typ == 6 && cls == 1) {
      has_soa = 1;
    }
    off += (int)rdlen;
  }
  if (has_soa && out->n_v4 == 0 && out->n_v6 == 0 && !out->has_cname) {
    out->nodata = 1;
  }
  if (rcode == 0 && !cache_hit && !cache_key.empty() &&
      (out->n_v4 || out->n_v6 || out->has_cname))
    EnvBoxDnsResponseCache::Store(cache_key, rbuf, rlen, GetTickCount64());
  return 1;
}

// Bootstrap uses only already-connectable Profile upstreams. In particular,
// another hostname DoH entry without addresses cannot recursively bootstrap.
// Reuse the normal wire identity/question checks and native record decoder;
// accept A addresses only for the requested name or its validated CNAME chain.
static int DnsBootstrapFromProfile(const char* name, char (&addresses)[ENVBOX_DNS_MAX][64],
                                  int* count, ULONGLONG deadline, HANDLE cancel_event) {
  *count = 0;
  const RuntimeProfile* profile = EnvBoxProfile();
  if (!profile || profile->dns_mode != 1) return 0;
  char current[256];
  strcpy_s(current, name);
  char visited[kDnsMaxCnameHops + 1][256] = {};
  strcpy_s(visited[0], current);
  int hops = 0;
  for (;;) {
    bool followed = false;
    for (int index = 0; index < DnsUpstreamCount(profile); ++index) {
      if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) return -1;
      if (GetTickCount64() >= deadline) return 0;
      DnsTransportEndpoint seed;
      char seed_name[256] = {};
      if (!DnsEndpoint(profile, index, &seed) || DnsDohBootstrapName(seed, seed_name)) continue;
      PDNS_RECORD records = nullptr;
      DNS_STATUS status = ERROR_TIMEOUT;
      int result = DnsQueryOne(seed, current, DNS_TYPE_A, DnsAttemptDeadline(deadline),
                               cancel_event, nullptr, &records, &status);
      if (result <= 0 || status != ERROR_SUCCESS) {
        if (records) TrueDnsFree(records, DnsFreeRecordList);
        if (result < 0) return -1;
        if (status == DNS_ERROR_RCODE_NAME_ERROR || status == DNS_INFO_NO_RECORDS) return 0;
        continue;
      }
      // A response can contain an entire alias chain in one answer section.
      for (;;) {
        if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) {
          TrueDnsFree(records, DnsFreeRecordList); return -1;
        }
        if (GetTickCount64() >= deadline) {
          TrueDnsFree(records, DnsFreeRecordList); return 0;
        }
        wchar_t owner[256] = {};
        if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, current, -1, owner, ARRAYSIZE(owner))) break;
        wchar_t* alias = nullptr;
        bool invalid = false;
        for (PDNS_RECORD record = records; record; record = record->pNext) {
          if (record->Flags.S.Section != DnsSectionAnswer || !record->pName ||
              _wcsicmp(reinterpret_cast<wchar_t*>(record->pName), owner) != 0) continue;
          if (record->wType == DNS_TYPE_A && *count < ENVBOX_DNS_MAX) {
            in_addr address;
            address.s_addr = record->Data.A.IpAddress;
            const ULONG host_address = ntohl(address.s_addr);
            if (host_address == 0 || host_address == INADDR_BROADCAST ||
                (host_address & 0xf0000000) == 0xe0000000) { invalid = true; break; }
            if (!InetNtopA(AF_INET, &address, addresses[*count], sizeof(addresses[*count]))) { invalid = true; break; }
            ++*count;
          } else if (record->wType == DNS_TYPE_CNAME) {
            wchar_t* target = reinterpret_cast<wchar_t*>(record->Data.PTR.pNameHost);
            if (!target || (alias && _wcsicmp(alias, target) != 0)) { invalid = true; break; }
            alias = target;
          }
        }
        if (invalid || (*count && alias)) { *count = 0; break; }
        if (*count) { TrueDnsFree(records, DnsFreeRecordList); return 1; }
        if (!alias) break;
        char target[256] = {};
        unsigned char encoded[256];
        if (hops == kDnsMaxCnameHops ||
            !WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, alias, -1, target, sizeof(target), nullptr, nullptr) ||
            !IsAsciiNameA(target) ||
            EncodeDnsName(target, encoded, sizeof(encoded)) <= 0) {
          TrueDnsFree(records, DnsFreeRecordList); return 0;
        }
        size_t target_length = strlen(target);
        if (target_length > 1 && target[target_length - 1] == '.') target[target_length - 1] = '\0';
        for (int previous = 0; previous <= hops; ++previous) {
          if (_stricmp(visited[previous], target) == 0) {
            TrueDnsFree(records, DnsFreeRecordList); return 0;
          }
        }
        strcpy_s(current, target);
        strcpy_s(visited[++hops], target);
        followed = true;
      }
      TrueDnsFree(records, DnsFreeRecordList);
      if (followed) break;
    }
    if (!followed) return 0;
  }
}

// Route one name through Profile servers in order. Follows CNAME (max
// kDnsMaxCnameHops). Returns:
//   1 = definitive answer (addresses, authoritative NXDOMAIN, or NODATA)
//   0 = no definitive answer (temporary resolution failure) -- truncated, unreachable, or
//       NOERROR without A/AAAA even after CNAME follow (including referral)
static int DnsRouteName(const char* qname, int want_a, int want_aaaa,
                        DnsAddrs* out, HANDLE cancel_event, ULONGLONG query_deadline = 0) {
  DnsAddrsClear(out);
  if (cancel_event != nullptr &&
      WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) {
    return -1;
  }
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || pfl->dns_mode != 1 || DnsUpstreamCount(pfl) <= 0) {
    return 0;
  }
  if (qname == nullptr || qname[0] == '\0') {
    return 0;
  }
  if (DnsCacheLookup(qname, want_a, want_aaaa, out)) {
    return 1;
  }

  // Balance the Runtime's Winsock reference with the complete bounded route.
  // DnsQueryOne closes every socket before returning, so no socket outlives
  // this scope. The scope is local to this call and therefore safe when
  // several Runtime threads resolve names concurrently.
  DnsWinsockScope winsock;
  if (!winsock.active) {
    return 0;
  }

  char current[256];
  size_t qn = strlen(qname);
  if (qn >= sizeof(current)) {
    return 0;
  }
  memcpy(current, qname, qn + 1);

  ULONGLONG deadline = GetTickCount64() + kDnsTotalBudgetMs;
  if (query_deadline != 0 && query_deadline < deadline) deadline = query_deadline;
  int n = DnsUpstreamCount(pfl);

  for (int hop = 0; hop < kDnsMaxCnameHops; hop++) {
    int saw_nx = 0;
    int saw_ok = 0;
    int saw_nodata_a = 0;
    int saw_nodata_aaaa = 0;
    int final_a = !want_a;
    int final_aaaa = !want_aaaa;
    int has_cname = 0;
    char next_name[256];
    next_name[0] = '\0';

    for (int si = 0; si < n; si++) {
      DnsTransportEndpoint endpoint;
      if (!DnsEndpoint(pfl, si, &endpoint)) continue;

      if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) return -1;
      if (!final_a) {
        ULONGLONG now = GetTickCount64();
        if (now >= deadline) {
          break;
        }
        DnsAddrs step;
        DnsAddrsClear(&step);
        int query_status =
            DnsQueryOne(endpoint, current, 1, DnsAttemptDeadline(deadline), cancel_event, &step);
        if (query_status < 0) {
          return -1;
        }
        if (query_status > 0) {
          if (step.nxdomain) {
            saw_nx = 1;
          } else if (step.noerror) {
            final_a = step.nodata || step.n_v4 > 0 || step.has_cname;
            if (step.nodata) {
              saw_nodata_a = 1;
            } else if (final_a) {
              saw_ok = 1;
            }
            for (int i = 0; i < step.n_v4 && out->n_v4 < kDnsMaxAnswers; i++) {
              out->v4[out->n_v4++] = step.v4[i];
            }
            if (step.has_cname && !has_cname) {
              has_cname = 1;
              memcpy(next_name, step.cname, sizeof(next_name));
            }
          }
        }
      }

      if (!final_aaaa && !saw_nx) {
        ULONGLONG now = GetTickCount64();
        if (now >= deadline) {
          break;
        }
        DnsAddrs step6;
        DnsAddrsClear(&step6);
        int query_status =
            DnsQueryOne(endpoint, current, 28, DnsAttemptDeadline(deadline), cancel_event, &step6);
        if (query_status < 0) {
          return -1;
        }
        if (query_status > 0) {
          if (step6.nxdomain) {
            saw_nx = 1;
          } else if (step6.noerror) {
            final_aaaa = step6.nodata || step6.n_v6 > 0 || step6.has_cname;
            if (step6.nodata) {
              saw_nodata_aaaa = 1;
            } else if (final_aaaa) {
              saw_ok = 1;
            }
            for (int i = 0; i < step6.n_v6 && out->n_v6 < kDnsMaxAnswers; i++) {
              out->v6[out->n_v6++] = step6.v6[i];
            }
            if (step6.has_cname && !has_cname) {
              has_cname = 1;
              memcpy(next_name, step6.cname, sizeof(next_name));
            }
          }
        }
      }

      // First definitive reply for this name wins (do not leak to Host DNS).
      if (saw_nx || (final_a && final_aaaa)) {
        break;
      }
    }

    if (saw_nx) {
      // Authoritative NXDOMAIN: only definitive "name does not exist".
      out->got = 1;
      out->nxdomain = 1;
      out->noerror = 0;
      DnsCacheStore(qname, want_a, want_aaaa, out);
      return 1;
    }
    if (out->n_v4 > 0 || out->n_v6 > 0) {
      out->got = 1;
      out->noerror = 1;
      out->nxdomain = 0;
      DnsCacheStore(qname, want_a, want_aaaa, out);
      return 1;
    }
    if ((want_a ? saw_nodata_a : 1) &&
        (want_aaaa ? saw_nodata_aaaa : 1)) {
      out->got = 1;
      out->noerror = 1;
      out->nxdomain = 0;
      out->nodata = 1;
      DnsCacheStore(qname, want_a, want_aaaa, out);
      return 1;
    }
    if (saw_ok && has_cname && next_name[0] != '\0') {
      // NOERROR + CNAME only: follow the chain.
      size_t nn = strlen(next_name);
      if (nn == 0 || nn >= sizeof(current)) {
        return 0;
      }
      memcpy(current, next_name, nn + 1);
      continue;
    }
    // NOERROR without A/AAAA and without usable CNAME, or no reply at all.
    // Not a name error -- report a temporary resolution failure.
    return 0;
  }
  // CNAME hops exhausted without addresses: temporary failure.
  return 0;
}

static int IsLocalMachineDnsNameW(const wchar_t* name);

static int IsLocalMachineDnsNameA(const char* name) {
  wchar_t wide[256];
  return name && MultiByteToWideChar(CP_UTF8, 0, name, -1, wide, ARRAYSIZE(wide)) &&
         IsLocalMachineDnsNameW(wide);
}

static int IsNumericNodeA(const char* n) {
  in_addr a4;
  in6_addr a6;
  if (n == nullptr || n[0] == '\0') {
    return 0;
  }
  return InetPtonA(AF_INET, n, &a4) == 1 || InetPtonA(AF_INET6, n, &a6) == 1;
}

static int IsNumericNodeW(const wchar_t* n) {
  in_addr a4;
  in6_addr a6;
  if (n == nullptr || n[0] == L'\0') {
    return 0;
  }
  return InetPtonW(AF_INET, n, &a4) == 1 || InetPtonW(AF_INET6, n, &a6) == 1;
}

static int IsAsciiNameA(const char* s) {
  for (; s && *s; s++) {
    if ((unsigned char)*s >= 0x80) {
      return 0;
    }
  }
  return 1;
}

// Host-order port from service string. Returns 1 on success.
static int ServiceToPort(const char* svc, int socktype, unsigned short* out) {
  *out = 0;
  if (svc == nullptr || svc[0] == '\0') {
    return 1;
  }
  char* end = nullptr;
  long p = strtol(svc, &end, 10);
  if (end != nullptr && *end == '\0' && p >= 0 && p <= 65535) {
    *out = (unsigned short)p;
    return 1;
  }
  struct servent* se =
      getservbyname(svc, socktype == SOCK_DGRAM ? "udp" : "tcp");
  if (se != nullptr) {
    *out = ntohs((unsigned short)se->s_port);
    return 1;
  }
  return 0;
}

static int WideToUtf8(const wchar_t* w, char* out, int cap) {
  int n = WideCharToMultiByte(CP_UTF8, 0, w, -1, out, cap, nullptr, nullptr);
  return n > 0;
}

struct AddrPair {
  int family;
  union {
    in_addr v4;
    in6_addr v6;
  } u;
};

static int CollectPairs(const DnsAddrs* in, int ai_family, AddrPair* out,
                        int cap) {
  int n = 0;
  if ((ai_family == AF_UNSPEC || ai_family == 0 || ai_family == AF_INET)) {
    for (int i = 0; i < in->n_v4 && n < cap; i++) {
      out[n].family = AF_INET;
      out[n].u.v4 = in->v4[i];
      n++;
    }
  }
  if ((ai_family == AF_UNSPEC || ai_family == 0 || ai_family == AF_INET6)) {
    for (int i = 0; i < in->n_v6 && n < cap; i++) {
      out[n].family = AF_INET6;
      out[n].u.v6 = in->v6[i];
      n++;
    }
  }
  return n;
}

static void FillSockaddr(const AddrPair* p, unsigned short port, sockaddr_storage* ss,
                         int* len) {
  memset(ss, 0, sizeof(*ss));
  if (p->family == AF_INET) {
    sockaddr_in v4;
    memset(&v4, 0, sizeof(v4));
    v4.sin_family = AF_INET;
    v4.sin_port = htons(port);
    v4.sin_addr = p->u.v4;
    memcpy(ss, &v4, sizeof(v4));
    *len = (int)sizeof(sockaddr_in);
  } else {
    sockaddr_in6 v6;
    memset(&v6, 0, sizeof(v6));
    v6.sin6_family = AF_INET6;
    v6.sin6_port = htons(port);
    v6.sin6_addr = p->u.v6;
    memcpy(ss, &v6, sizeof(v6));
    *len = (int)sizeof(sockaddr_in6);
  }
}

// Create one ADDRINFOA node. Returns nullptr on OOM.
static PADDRINFOA AllocAddrInfoA(const AddrPair* p, unsigned short port,
                                 int socktype, int protocol, int flags,
                                 const char* canon) {
  PADDRINFOA ai = (PADDRINFOA)OwnedAlloc(sizeof(ADDRINFOA));
  if (ai == nullptr) {
    return nullptr;
  }
  sockaddr_storage ss;
  int slen = 0;
  FillSockaddr(p, port, &ss, &slen);
  ai->ai_addr = (sockaddr*)OwnedAlloc(sizeof(sockaddr_storage));
  if (ai->ai_addr == nullptr) {
    OwnedFree(ai);
    return nullptr;
  }
  memcpy(ai->ai_addr, &ss, (size_t)slen);
  ai->ai_addrlen = (size_t)slen;
  ai->ai_family = p->family;
  ai->ai_socktype = socktype;
  ai->ai_protocol = protocol;
  ai->ai_flags = flags;
  ai->ai_next = nullptr;
  if (canon != nullptr && canon[0] != '\0') {
    size_t n = strlen(canon) + 1;
    ai->ai_canonname = (char*)OwnedAlloc(n);
    if (ai->ai_canonname != nullptr) {
      memcpy(ai->ai_canonname, canon, n);
    }
  }
  return ai;
}

static PADDRINFOW AllocAddrInfoW(const AddrPair* p, unsigned short port,
                                 int socktype, int protocol, int flags,
                                 const wchar_t* canon) {
  PADDRINFOW ai = (PADDRINFOW)OwnedAlloc(sizeof(ADDRINFOW));
  if (ai == nullptr) {
    return nullptr;
  }
  sockaddr_storage ss;
  int slen = 0;
  FillSockaddr(p, port, &ss, &slen);
  ai->ai_addr = (sockaddr*)OwnedAlloc(sizeof(sockaddr_storage));
  if (ai->ai_addr == nullptr) {
    OwnedFree(ai);
    return nullptr;
  }
  memcpy(ai->ai_addr, &ss, (size_t)slen);
  ai->ai_addrlen = (size_t)slen;
  ai->ai_family = p->family;
  ai->ai_socktype = socktype;
  ai->ai_protocol = protocol;
  ai->ai_flags = flags;
  ai->ai_next = nullptr;
  if (canon != nullptr && canon[0] != L'\0') {
    size_t n = wcslen(canon) + 1;
    ai->ai_canonname = (wchar_t*)OwnedAlloc(n * sizeof(wchar_t));
    if (ai->ai_canonname != nullptr) {
      memcpy(ai->ai_canonname, canon, n * sizeof(wchar_t));
    }
  }
  return ai;
}

static PADDRINFOEXW AllocAddrInfoExW(const AddrPair* p, unsigned short port,
                                     int socktype, int protocol, int flags,
                                     const wchar_t* canon) {
  PADDRINFOEXW ai = (PADDRINFOEXW)OwnedAlloc(sizeof(ADDRINFOEXW));
  if (ai == nullptr) {
    return nullptr;
  }
  sockaddr_storage ss;
  int slen = 0;
  FillSockaddr(p, port, &ss, &slen);
  ai->ai_addr = (sockaddr*)OwnedAlloc(sizeof(sockaddr_storage));
  if (ai->ai_addr == nullptr) {
    OwnedFree(ai);
    return nullptr;
  }
  memcpy(ai->ai_addr, &ss, (size_t)slen);
  ai->ai_addrlen = (size_t)slen;
  ai->ai_family = p->family;
  ai->ai_socktype = socktype;
  ai->ai_protocol = protocol;
  ai->ai_flags = flags;
  ai->ai_next = nullptr;
  ai->ai_blob = nullptr;
  ai->ai_bloblen = 0;
  ai->ai_provider = nullptr;
  if (canon != nullptr && canon[0] != L'\0') {
    size_t n = wcslen(canon) + 1;
    ai->ai_canonname = (wchar_t*)OwnedAlloc(n * sizeof(wchar_t));
    if (ai->ai_canonname != nullptr) {
      memcpy(ai->ai_canonname, canon, n * sizeof(wchar_t));
    }
  }
  return ai;
}

static PADDRINFOEXA AllocAddrInfoExA(const AddrPair* p, unsigned short port,
                                     int socktype, int protocol, int flags,
                                     const char* canon) {
  PADDRINFOEXA ai = (PADDRINFOEXA)OwnedAlloc(sizeof(ADDRINFOEXA));
  if (ai == nullptr) {
    return nullptr;
  }
  sockaddr_storage ss;
  int slen = 0;
  FillSockaddr(p, port, &ss, &slen);
  ai->ai_addr = (sockaddr*)OwnedAlloc(sizeof(sockaddr_storage));
  if (ai->ai_addr == nullptr) {
    OwnedFree(ai);
    return nullptr;
  }
  memcpy(ai->ai_addr, &ss, (size_t)slen);
  ai->ai_addrlen = (size_t)slen;
  ai->ai_family = p->family;
  ai->ai_socktype = socktype;
  ai->ai_protocol = protocol;
  ai->ai_flags = flags;
  ai->ai_next = nullptr;
  ai->ai_blob = nullptr;
  ai->ai_bloblen = 0;
  ai->ai_provider = nullptr;
  if (canon != nullptr && canon[0] != '\0') {
    size_t n = strlen(canon) + 1;
    ai->ai_canonname = (char*)OwnedAlloc(n);
    if (ai->ai_canonname != nullptr) {
      memcpy(ai->ai_canonname, canon, n);
    }
  }
  return ai;
}

// Expand each address into TCP/UDP nodes per getaddrinfo conventions.
template <typename TNode, typename TCanon, typename TAlloc>
static TNode* BuildChain(const AddrPair* pairs, int np, unsigned short port,
                         int ai_family, int ai_socktype, int ai_protocol,
                         int ai_flags, const TCanon* canon, TAlloc alloc) {
  (void)ai_family;
  TNode* head = nullptr;
  TNode* tail = nullptr;
  int want_types[2];
  int want_protos[2];
  int ntypes = 0;
  if (ai_socktype == 0 && ai_protocol == 0) {
    want_types[0] = SOCK_STREAM;
    want_protos[0] = IPPROTO_TCP;
    want_types[1] = SOCK_DGRAM;
    want_protos[1] = IPPROTO_UDP;
    ntypes = 2;
  } else if (ai_socktype == SOCK_STREAM) {
    want_types[0] = SOCK_STREAM;
    want_protos[0] = ai_protocol ? ai_protocol : IPPROTO_TCP;
    ntypes = 1;
  } else if (ai_socktype == SOCK_DGRAM) {
    want_types[0] = SOCK_DGRAM;
    want_protos[0] = ai_protocol ? ai_protocol : IPPROTO_UDP;
    ntypes = 1;
  } else {
    want_types[0] = ai_socktype;
    want_protos[0] = ai_protocol;
    ntypes = 1;
  }

  int first = 1;
  for (int i = 0; i < np; i++) {
    for (int t = 0; t < ntypes; t++) {
      const TCanon* cn = nullptr;
      if (first && (ai_flags & AI_CANONNAME)) {
        cn = canon;
        first = 0;
      }
      TNode* node =
          alloc(&pairs[i], port, want_types[t], want_protos[t], ai_flags, cn);
      if (node == nullptr) {
        while (head != nullptr) {
          TNode* n = head->ai_next;
          // Owned nodes are freed through free path below.
          if (head->ai_canonname) {
            OwnedFree(head->ai_canonname);
          }
          if (head->ai_addr) {
            OwnedFree(head->ai_addr);
          }
          OwnedFree(head);
          head = n;
        }
        return nullptr;
      }
      if (tail == nullptr) {
        head = node;
      } else {
        tail->ai_next = node;
      }
      tail = node;
    }
  }
  return head;
}

// Shared resolve for ANSI/Wide entry. qname_utf8 is ASCII LDH.
// Returns EAI_* or 0. On 0, *out_chain owns a chain (may be empty family-filtered).
static int ResolveRoutedA(const char* qname_utf8, const char* service,
                          int ai_family, int ai_socktype, int ai_protocol,
                          int ai_flags, DnsAddrs* addrs, unsigned short* port,
                          ULONGLONG query_deadline = 0,
                          HANDLE cancel_event = nullptr) {
  int want_a = (ai_family == AF_UNSPEC || ai_family == 0 || ai_family == AF_INET);
  int want_aaaa =
      (ai_family == AF_UNSPEC || ai_family == 0 || ai_family == AF_INET6);
  if (!want_a && !want_aaaa) {
    return EAI_FAMILY;
  }

  unsigned short p = 0;
  int probe_type = (ai_socktype == SOCK_DGRAM) ? SOCK_DGRAM : SOCK_STREAM;
  if (!ServiceToPort(service, probe_type, &p)) {
    return EAI_SERVICE;
  }
  *port = p;

  if (DnsRouteName(qname_utf8, want_a, want_aaaa, addrs, cancel_event,
                   query_deadline) <= 0) {
    return EAI_AGAIN;  // no Host fallback
  }
  if ((addrs->nxdomain || addrs->nodata) && addrs->n_v4 == 0 &&
      addrs->n_v6 == 0) {
    return EAI_NONAME;
  }
  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(addrs, ai_family, pairs, kDnsMaxAnswers * 2);
  if (np == 0) {
    if (addrs->n_v4 > 0 || addrs->n_v6 > 0) {
      // Addresses exist but none match ai_family.
      return EAI_NONAME;
    }
    // NOERROR without A/AAAA (CNAME-only / other type): temporary failure, never
    // fabricate NXDOMAIN.
    return EAI_AGAIN;
  }
  return 0;
}

// Owned free for self-built chains. Never True*free HeapAlloc nodes.
static void FreeAddrInfoAChain(PADDRINFOA ai) {
  while (ai != nullptr) {
    PADDRINFOA next = ai->ai_next;
    if (ai->ai_canonname != nullptr) {
      OwnedFree(ai->ai_canonname);
    }
    if (ai->ai_addr != nullptr) {
      OwnedFree(ai->ai_addr);
    }
    OwnedFree(ai);
    ai = next;
  }
}

static void FreeAddrInfoWChain(PADDRINFOW ai) {
  while (ai != nullptr) {
    PADDRINFOW next = ai->ai_next;
    if (ai->ai_canonname != nullptr) {
      OwnedFree(ai->ai_canonname);
    }
    if (ai->ai_addr != nullptr) {
      OwnedFree(ai->ai_addr);
    }
    OwnedFree(ai);
    ai = next;
  }
}

static void FreeAddrInfoExAChain(PADDRINFOEXA ai) {
  while (ai != nullptr) {
    PADDRINFOEXA next = ai->ai_next;
    if (ai->ai_canonname != nullptr) {
      OwnedFree(ai->ai_canonname);
    }
    if (ai->ai_addr != nullptr) {
      OwnedFree(ai->ai_addr);
    }
    if (ai->ai_blob != nullptr) {
      OwnedFree(ai->ai_blob);
    }
    OwnedFree(ai);
    ai = next;
  }
}

static void FreeAddrInfoExWChain(PADDRINFOEXW ai) {
  while (ai != nullptr) {
    PADDRINFOEXW next = ai->ai_next;
    if (ai->ai_canonname != nullptr) {
      OwnedFree(ai->ai_canonname);
    }
    if (ai->ai_addr != nullptr) {
      OwnedFree(ai->ai_addr);
    }
    if (ai->ai_blob != nullptr) {
      OwnedFree(ai->ai_blob);
    }
    OwnedFree(ai);
    ai = next;
  }
}

// GetAddrInfoExW is the only GetAddrInfoEx ABI that supports asynchronous
// parameters on the Windows versions supported by this Runtime.  Do not pass
// a Profile query to the native provider: its namespace policy can ignore the
// caller's server view.  The state below owns the worker and cancellation
// event, while the caller retains the documented OVERLAPPED/result storage
// until completion. The native status-only result helper reads the published
// OVERLAPPED state even after the worker retires its internal map entry.
struct EnvBoxAddrInfoAsyncContext {
  LPLOOKUPSERVICE_COMPLETION_ROUTINE completion;
  LPOVERLAPPED overlapped;
  PADDRINFOEXW* result;
  HANDLE cancel_token;
  HANDLE cancel_event;
  HANDLE notify_event;
  DWORD name_space;
  ULONGLONG query_deadline;
  wchar_t query_name[256];
  wchar_t service[64];
  ADDRINFOEXW hints;
  int has_hints;
  volatile LONG cancel_requested;
  volatile LONG refs;
  int completion_started;
  int completed;
  INT completed_status;
  int registered;
  EnvBoxAddrInfoAsyncContext* next;
};

static const LONG kAddrInfoAsyncMaxPending = 64;
static const size_t kAddrInfoAsyncNameCapacity = 256;
static const size_t kAddrInfoAsyncServiceCapacity = 64;
// The Profile wire route can preserve these legacy address-info semantics.
// Flags that request Windows cache/LLMNR/custom-server/secure-DNS behavior
// are rejected rather than silently changing their meaning on the wire.
static const int kAddrInfoSupportedFlags =
    AI_PASSIVE | AI_CANONNAME | AI_NUMERICHOST;
static int AddrInfoFlagsSupported(int flags) {
  return (flags & ~kAddrInfoSupportedFlags) == 0;
}
static volatile LONG64 g_addr_info_async_generation = 0;
static LONG g_addr_info_async_work_count = 0;
static EnvBoxAddrInfoAsyncContext* g_addr_info_async_pending = nullptr;
static SRWLOCK g_addr_info_async_lock = SRWLOCK_INIT;

static HANDLE AddrInfoAsyncToken(ULONGLONG generation) {
  // The token is an opaque value consumed only by our Cancel hook.  Keeping it
  // out of the kernel handle table avoids a close/reuse race when a caller
  // retains a stale lpNameHandle after completion.
  ULONG_PTR value = static_cast<ULONG_PTR>((generation << 1) | 1ULL);
  return reinterpret_cast<HANDLE>(value);
}

static ULONGLONG NextAddrInfoAsyncGeneration() {
  LONG64 generation = InterlockedIncrement64(&g_addr_info_async_generation);
  if (generation <= 0) {
    InterlockedExchange64(&g_addr_info_async_generation, 1);
    generation = 1;
  }
  return static_cast<ULONGLONG>(generation);
}

static EnvBoxAddrInfoAsyncContext* FindAddrInfoAsyncByTokenLocked(
    HANDLE token) {
  for (EnvBoxAddrInfoAsyncContext* p = g_addr_info_async_pending;
       p != nullptr; p = p->next) {
    if (p->cancel_token == token) return p;
  }
  return nullptr;
}

static int RegisterAddrInfoAsync(EnvBoxAddrInfoAsyncContext* context) {
  int ok = 0;
  AcquireSRWLockExclusive(&g_addr_info_async_lock);
  if (g_addr_info_async_work_count < kAddrInfoAsyncMaxPending) {
    g_addr_info_async_work_count++;
    // One reference belongs to the pending-map entry and one to the queued
    // worker.  The worker reference prevents event/callback consumers from
    // freeing the context while the worker is still publishing completion.
    context->refs = 2;
    context->registered = 1;
    context->next = g_addr_info_async_pending;
    g_addr_info_async_pending = context;
    ok = 1;
  }
  ReleaseSRWLockExclusive(&g_addr_info_async_lock);
  return ok;
}

static void ReleaseAddrInfoAsyncRef(EnvBoxAddrInfoAsyncContext* context);

static int UnregisterAddrInfoAsync(EnvBoxAddrInfoAsyncContext* context) {
  if (context == nullptr) return 0;
  int removed = 0;
  AcquireSRWLockExclusive(&g_addr_info_async_lock);
  if (context->registered) {
    EnvBoxAddrInfoAsyncContext** link = &g_addr_info_async_pending;
    while (*link != nullptr) {
      if (*link == context) {
        *link = context->next;
        break;
      }
      link = &(*link)->next;
    }
    context->registered = 0;
    if (g_addr_info_async_work_count > 0) g_addr_info_async_work_count--;
    removed = 1;
  }
  ReleaseSRWLockExclusive(&g_addr_info_async_lock);
  if (removed) ReleaseAddrInfoAsyncRef(context);
  return removed;
}

static void ReleaseAddrInfoAsyncRef(EnvBoxAddrInfoAsyncContext* context) {
  if (context == nullptr || InterlockedDecrement(&context->refs) != 0) {
    return;
  }
  if (context->cancel_event != nullptr) {
    CloseHandle(context->cancel_event);
    context->cancel_event = nullptr;
  }
  if (context->notify_event != nullptr) {
    CloseHandle(context->notify_event);
    context->notify_event = nullptr;
  }
  HeapFree(GetProcessHeap(), 0, context);
}

static void CompleteAddrInfoAsync(EnvBoxAddrInfoAsyncContext* context,
                                  INT status, PADDRINFOEXW records,
                                  int native_records) {
  if (context == nullptr) return;
  int should_notify = 0;
  int cancelled = 0;
  AcquireSRWLockExclusive(&g_addr_info_async_lock);
  if (!context->registered || context->completion_started) {
    ReleaseSRWLockExclusive(&g_addr_info_async_lock);
    if (records != nullptr) {
      if (native_records) {
        TrueFreeAddrInfoExW(records);
      } else {
        FreeAddrInfoExWChain(records);
      }
    }
    ReleaseAddrInfoAsyncRef(context);
    return;
  }
  context->completion_started = 1;
  cancelled = InterlockedCompareExchange(&context->cancel_requested, 0, 0) != 0;
  if (cancelled) {
    status = WSA_E_CANCELLED;
    if (records != nullptr) {
      if (native_records) {
        TrueFreeAddrInfoExW(records);
      } else {
        FreeAddrInfoExWChain(records);
      }
      records = nullptr;
    }
  }
  context->completed_status = status;
  // Complete all writes through caller-owned storage before publishing the
  // terminal status read by the native result helper. lpNameHandle itself is
  // only an output slot: after returning from GetAddrInfoExW the caller may
  // discard it, so the context never retains or rewrites that pointer.
  if (context->result != nullptr) *context->result = records;
  // Keep the documented result-slot pointer in OVERLAPPED.Pointer as well;
  // the native helper reads this publication on Windows builds whose export
  // is too short for a Detours trampoline.  InternalHigh remains zero, as it
  // does for the native provider; the helper is a status read, not an EnvBox
  // marker protocol.
  context->overlapped->InternalHigh = 0;
  context->overlapped->Pointer = reinterpret_cast<PVOID>(context->result);
  LPLOOKUPSERVICE_COMPLETION_ROUTINE completion = context->completion;
  LPOVERLAPPED overlapped = context->overlapped;
  HANDLE event = context->notify_event;
  context->completed = 1;
  // Native GetAddrInfoExOverlappedResult does not take our map lock. Its status
  // read can let the caller release OVERLAPPED immediately, so this is the last
  // access to borrowed output and must publish all preceding result writes.
  InterlockedExchangePointer(
      reinterpret_cast<PVOID volatile*>(&overlapped->Internal),
      reinterpret_cast<PVOID>(static_cast<ULONG_PTR>(status)));
  should_notify = 1;
  ReleaseSRWLockExclusive(&g_addr_info_async_lock);

  if (!should_notify) return;
  if (completion != nullptr) {
    // The context remains registered during user code so a reentrant Cancel
    // sees a completed operation and cannot free it twice.
    completion(static_cast<DWORD>(status), 0, overlapped);
    UnregisterAddrInfoAsync(context);
    ReleaseAddrInfoAsyncRef(context);
  } else if (event != nullptr) {
    SetEvent(event);
    // The helper needs no context; retire even when callers never query it.
    UnregisterAddrInfoAsync(context);
    ReleaseAddrInfoAsyncRef(context);
  } else {
    // This is prevented before queueing, but leave a deterministic failure if
    // a malformed OVERLAPPED is ever observed after registration.
    UnregisterAddrInfoAsync(context);
    ReleaseAddrInfoAsyncRef(context);
  }
}

static DWORD WINAPI AddrInfoAsyncWorker(PVOID parameter) {
  EnvBoxAddrInfoAsyncContext* context =
      static_cast<EnvBoxAddrInfoAsyncContext*>(parameter);
  if (context == nullptr) return 0;

  PADDRINFOEXW records = nullptr;
  INT status = EAI_AGAIN;
  int native_records = 0;
  const int numeric_or_local =
      context->query_name[0] == L'\0' ||
      IsNumericNodeW(context->query_name) ||
      IsLocalMachineDnsNameW(context->query_name) ||
      (context->has_hints &&
       (context->hints.ai_flags & AI_NUMERICHOST) != 0);
  if (numeric_or_local) {
    const ADDRINFOEXW* hints = context->has_hints ? &context->hints : nullptr;
    status = TrueGetAddrInfoExW(
        context->query_name,
        context->service[0] == L'\0' ? nullptr : context->service,
        context->name_space, nullptr, hints, &records, nullptr, nullptr,
        nullptr, nullptr);
    native_records = records != nullptr;
  } else {
    char name_u8[256] = {};
    char service_u8[64] = {};
    if (WideToUtf8(context->query_name, name_u8, ARRAYSIZE(name_u8)) &&
        (context->service[0] == L'\0' ||
         WideToUtf8(context->service, service_u8, ARRAYSIZE(service_u8)))) {
      const ADDRINFOEXW* hints = context->has_hints ? &context->hints : nullptr;
      const int flags = hints ? hints->ai_flags : 0;
      const int family = hints ? hints->ai_family : AF_UNSPEC;
      const int stype = hints ? hints->ai_socktype : 0;
      const int proto = hints ? hints->ai_protocol : 0;
      DnsAddrs addrs;
      unsigned short port = 0;
      int routed = ResolveRoutedA(
          name_u8, context->service[0] == L'\0' ? nullptr : service_u8,
          family, stype, proto, flags, &addrs, &port,
          context->query_deadline, context->cancel_event);
      if (routed == 0) {
        AddrPair pairs[kDnsMaxAnswers * 2];
        int np = CollectPairs(&addrs, family, pairs, ARRAYSIZE(pairs));
        records = BuildChain<ADDRINFOEXW, wchar_t, decltype(&AllocAddrInfoExW)>(
            pairs, np, port, family, stype, proto, flags, context->query_name,
            &AllocAddrInfoExW);
        status = records == nullptr ? EAI_MEMORY : 0;
      } else if (routed == EAI_NONAME) {
        status = EAI_NONAME;
      } else {
        status = routed;
      }
    }
  }
  CompleteAddrInfoAsync(context, status, records, native_records);
  return 0;
}

static INT WSAAPI HookGetAddrInfoExCancel(LPHANDLE handle) {
  DWORD err = GetLastError();
  if (handle == nullptr || *handle == nullptr) {
    SetLastError(err);
    return WSA_INVALID_HANDLE;
  }
  AcquireSRWLockExclusive(&g_addr_info_async_lock);
  EnvBoxAddrInfoAsyncContext* context =
      FindAddrInfoAsyncByTokenLocked(*handle);
  if (context == nullptr) {
    ReleaseSRWLockExclusive(&g_addr_info_async_lock);
    if (g_view_active) {
      // Every VirtualView GetAddrInfoEx async entrypoint is either owned by
      // this map or rejected before calling the native provider (ExA async
      // arguments and ExW namespace/provider inputs).  Passing an unknown
      // token through here could therefore cancel an operation whose DNS
      // view we do not control.  Host mode never installs this detour and
      // keeps the native cancellation contract unchanged.
      EnvBoxAuditEvent("GetAddrInfoExCancel", 0, "dns-virtual-stale");
      SetLastError(err);
      return WSA_INVALID_HANDLE;
    }
    INT status = TrueGetAddrInfoExCancel(handle);
    SetLastError(err);
    return status;
  }
  if (context->completion_started || context->completed) {
    ReleaseSRWLockExclusive(&g_addr_info_async_lock);
    EnvBoxAuditEvent("GetAddrInfoExCancel", 0, "dns-virtual-completed");
    SetLastError(err);
    return WSA_INVALID_HANDLE;
  }
  InterlockedExchange(&context->cancel_requested, 1);
  BOOL signaled = SetEvent(context->cancel_event);
  ReleaseSRWLockExclusive(&g_addr_info_async_lock);
  EnvBoxAuditEvent("GetAddrInfoExCancel", signaled ? 1 : 0,
                   signaled ? "dns-virtual-cancel" : "dns-virtual-cancel-failed");
  SetLastError(err);
  return signaled ? NO_ERROR : WSAEINTR;
}

static int HookGetAddrInfoCommonA(PCSTR node, PCSTR service,
                                  const ADDRINFOA* hints, PADDRINFOA* result,
                                  const char* api) {
  DWORD err = GetLastError();
  if (result != nullptr) {
    *result = nullptr;
  }
  if (!g_view_active) {
    INT r = Truegetaddrinfo(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "dns-host");
    SetLastError(err);
    return r;
  }
  int flags = hints ? hints->ai_flags : 0;
  int family = hints ? hints->ai_family : AF_UNSPEC;
  int stype = hints ? hints->ai_socktype : 0;
  int proto = hints ? hints->ai_protocol : 0;
  if (!AddrInfoFlagsSupported(flags)) {
    EnvBoxAuditEvent(api, 1, "dns-unsupported-flags");
    SetLastError(err);
    return WSAEOPNOTSUPP;
  }

  // Numeric, local-machine and service-only inputs do not need a DNS route.
  if (node && !IsAsciiNameA(node) && !IsLocalMachineDnsNameA(node)) {
    EnvBoxAuditEvent(api, 1, "dns-unsupported-name");
    SetLastError(err);
    return EAI_FAIL;
  }
  if (node == nullptr || node[0] == '\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeA(node) || IsLocalMachineDnsNameA(node)) {
    LocalhostResolutionScope local(node);
    INT r = Truegetaddrinfo(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "numeric-or-passthrough");
    SetLastError(err);
    return r;
  }

  DnsAddrs addrs;
  unsigned short port = 0;
  int rc = ResolveRoutedA(node, service, family, stype, proto, flags, &addrs,
                          &port);
  if (rc == EAI_AGAIN) {
    EnvBoxAuditEvent(api, 1, "dns-profile-error");
    SetLastError(err);
    return EAI_AGAIN;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    const char* kind = addrs.nodata ? "nodata" : "nxdomain";
    _snprintf_s(note, sizeof(note), _TRUNCATE, "%s node=%.64s", kind, node);
    EnvBoxAuditEvent(api, 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    EnvBoxAuditEvent(api, 1, "dns-profile-error");
    SetLastError(err);
    return rc;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOA chain = BuildChain<ADDRINFOA, char, decltype(&AllocAddrInfoA)>(
      pairs, np, port, family, stype, proto, flags, node, &AllocAddrInfoA);
  if (chain == nullptr) {
    EnvBoxAuditEvent(api, 1, "dns-profile-error");
    SetLastError(err);
    return EAI_MEMORY;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoAChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route n=%d", np);
  EnvBoxAuditEvent(api, 1, note);
  SetLastError(err);
  return 0;
}

static int HookGetAddrInfoCommonW(PCWSTR node, PCWSTR service,
                                  const ADDRINFOW* hints, PADDRINFOW* result,
                                  const char* api) {
  DWORD err = GetLastError();
  if (result != nullptr) {
    *result = nullptr;
  }
  if (!g_view_active) {
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "dns-host");
    SetLastError(err);
    return r;
  }
  int flags = hints ? hints->ai_flags : 0;
  int family = hints ? hints->ai_family : AF_UNSPEC;
  int stype = hints ? hints->ai_socktype : 0;
  int proto = hints ? hints->ai_protocol : 0;
  if (!AddrInfoFlagsSupported(flags)) {
    EnvBoxAuditEvent(api, 1, "dns-unsupported-flags");
    SetLastError(err);
    return WSAEOPNOTSUPP;
  }

  if (node == nullptr || node[0] == L'\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeW(node) || IsLocalMachineDnsNameW(node)) {
    LocalhostResolutionScope local(node);
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "numeric-or-passthrough");
    SetLastError(err);
    return r;
  }

  char node_u8[256];
  if (!WideToUtf8(node, node_u8, (int)sizeof(node_u8)) ||
      !IsAsciiNameA(node_u8)) {
    EnvBoxAuditEvent(api, 1, "dns-unsupported-name");
    SetLastError(err);
    return EAI_FAIL;
  }
  char svc_u8[64];
  svc_u8[0] = '\0';
  if (service != nullptr && service[0] != L'\0') {
    if (!WideToUtf8(service, svc_u8, (int)sizeof(svc_u8))) {
      EnvBoxAuditEvent(api, 1, "dns-unsupported-service");
      SetLastError(err);
      return EAI_SERVICE;
    }
  }

  DnsAddrs addrs;
  unsigned short port = 0;
  int rc = ResolveRoutedA(node_u8, svc_u8, family, stype, proto, flags, &addrs,
                          &port);
  if (rc == EAI_AGAIN) {
    EnvBoxAuditEvent(api, 1, "dns-profile-error");
    SetLastError(err);
    return EAI_AGAIN;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    const char* kind = addrs.nodata ? "nodata" : "nxdomain";
    _snprintf_s(note, sizeof(note), _TRUNCATE, "%s node=%.64s", kind,
                node_u8);
    EnvBoxAuditEvent(api, 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    EnvBoxAuditEvent(api, 1, "dns-profile-error");
    SetLastError(err);
    return rc;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOW chain = BuildChain<ADDRINFOW, wchar_t, decltype(&AllocAddrInfoW)>(
      pairs, np, port, family, stype, proto, flags, node, &AllocAddrInfoW);
  if (chain == nullptr) {
    EnvBoxAuditEvent(api, 1, "dns-profile-error");
    SetLastError(err);
    return EAI_MEMORY;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoWChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route n=%d", np);
  EnvBoxAuditEvent(api, 1, note);
  SetLastError(err);
  return 0;
}

static INT WSAAPI Hookgetaddrinfo(PCSTR node, PCSTR service,
                                  const ADDRINFOA* hints, PADDRINFOA* result) {
  return HookGetAddrInfoCommonA(node, service, hints, result, "getaddrinfo");
}

static INT WSAAPI HookGetAddrInfoW(PCWSTR node, PCWSTR service,
                                   const ADDRINFOW* hints,
                                   PADDRINFOW* result) {
  return HookGetAddrInfoCommonW(node, service, hints, result, "GetAddrInfoW");
}

static void WSAAPI Hookfreeaddrinfo(PADDRINFOA ai) {
  DWORD err = GetLastError();
  if (!OwnedHas(ai)) {
    Truefreeaddrinfo(ai);
    SetLastError(err);
    return;
  }
  FreeAddrInfoAChain(ai);
  SetLastError(err);
}

static void WSAAPI HookFreeAddrInfoW(PADDRINFOW ai) {
  DWORD err = GetLastError();
  if (!OwnedHas(ai)) {
    TrueFreeAddrInfoW(ai);
    SetLastError(err);
    return;
  }
  FreeAddrInfoWChain(ai);
  SetLastError(err);
}

static void WSAAPI HookFreeAddrInfoExA(PADDRINFOEXA ai) {
  DWORD err = GetLastError();
  if (!OwnedHas(ai)) {
    TrueFreeAddrInfoExA(ai);
    SetLastError(err);
    return;
  }
  FreeAddrInfoExAChain(ai);
  SetLastError(err);
}

static void WSAAPI HookFreeAddrInfoExW(PADDRINFOEXW ai) {
  DWORD err = GetLastError();
  if (!OwnedHas(ai)) {
    TrueFreeAddrInfoExW(ai);
    SetLastError(err);
    return;
  }
  FreeAddrInfoExWChain(ai);
  SetLastError(err);
}

static INT WSAAPI HookGetAddrInfoExA(
    PCSTR name, PCSTR service, DWORD dw_name_space, LPGUID nlp_id,
    const ADDRINFOEXA* hints, PADDRINFOEXA* result, struct timeval* timeout,
    LPOVERLAPPED overlapped,
    LPLOOKUPSERVICE_COMPLETION_ROUTINE completion, LPHANDLE name_handle) {
  DWORD err = GetLastError();
  if (g_view_active && (overlapped != nullptr || completion != nullptr ||
      timeout != nullptr || name_handle != nullptr || nlp_id != nullptr ||
      (dw_name_space != 0 && dw_name_space != NS_ALL && dw_name_space != NS_DNS))) {
    if (result != nullptr) *result = nullptr;
    EnvBoxAuditEvent("GetAddrInfoExA", 1, "dns-unsupported-provider-or-async");
    SetLastError(err);
    return WSAEOPNOTSUPP;
  }
  // This is the Host-mode/native path; VirtualView async input was rejected
  // above before a provider could create an unowned pending operation.
  if (overlapped != nullptr || completion != nullptr) {
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, "fail-open-async");
    SetLastError(err);
    return r;
  }
  if (dw_name_space != 0 && dw_name_space != NS_ALL && dw_name_space != NS_DNS) {
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, "fail-open");
    SetLastError(err);
    return r;
  }

  if (!g_view_active) {
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, "dns-host");
    SetLastError(err);
    return r;
  }

  int flags = hints ? hints->ai_flags : 0;
  int family = hints ? hints->ai_family : AF_UNSPEC;
  int stype = hints ? hints->ai_socktype : 0;
  int proto = hints ? hints->ai_protocol : 0;
  if (!AddrInfoFlagsSupported(flags)) {
    EnvBoxAuditEvent("GetAddrInfoExA", 1, "dns-unsupported-flags");
    SetLastError(err);
    return WSAEOPNOTSUPP;
  }

  if (name && !IsAsciiNameA(name) && !IsLocalMachineDnsNameA(name)) {
    EnvBoxAuditEvent("GetAddrInfoExA", 1, "dns-unsupported-name");
    SetLastError(err);
    return EAI_FAIL;
  }
  if (name == nullptr || name[0] == '\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeA(name) || IsLocalMachineDnsNameA(name)) {
    LocalhostResolutionScope local(name);
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, "numeric-or-passthrough");
    SetLastError(err);
    return r;
  }

  DnsAddrs addrs;
  unsigned short port = 0;
  int rc = ResolveRoutedA(name, service, family, stype, proto, flags, &addrs,
                          &port);
  if (rc == EAI_AGAIN) {
    EnvBoxAuditEvent("GetAddrInfoExA", 1, "dns-profile-error");
    SetLastError(err);
    return EAI_AGAIN;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    const char* kind = addrs.nodata ? "nodata" : "nxdomain";
    _snprintf_s(note, sizeof(note), _TRUNCATE, "%s node=%.64s", kind, name);
    EnvBoxAuditEvent("GetAddrInfoExA", 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    EnvBoxAuditEvent("GetAddrInfoExA", 1, "dns-profile-error");
    SetLastError(err);
    return rc;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOEXA chain =
      BuildChain<ADDRINFOEXA, char, decltype(&AllocAddrInfoExA)>(
          pairs, np, port, family, stype, proto, flags, name,
          &AllocAddrInfoExA);
  if (chain == nullptr) {
    EnvBoxAuditEvent("GetAddrInfoExA", 1, "dns-profile-error");
    SetLastError(err);
    return EAI_MEMORY;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoExAChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route n=%d", np);
  EnvBoxAuditEvent("GetAddrInfoExA", 1, note);
  SetLastError(err);
  return 0;
}

static INT StartGetAddrInfoExWAsync(
    PCWSTR name, PCWSTR service, DWORD dw_name_space, LPGUID nlp_id,
    const ADDRINFOEXW* hints, PADDRINFOEXW* result, struct timeval* timeout,
    LPOVERLAPPED overlapped,
    LPLOOKUPSERVICE_COMPLETION_ROUTINE completion, LPHANDLE name_handle) {
  if (result != nullptr) *result = nullptr;
  if (name_handle != nullptr) *name_handle = nullptr;
  if (nlp_id != nullptr ||
      (dw_name_space != 0 && dw_name_space != NS_ALL &&
       dw_name_space != NS_DNS)) {
    return WSAEOPNOTSUPP;
  }
  if (result == nullptr || overlapped == nullptr ||
      (completion == nullptr && overlapped->hEvent == nullptr) ||
      (completion != nullptr && overlapped->hEvent != nullptr)) {
    return WSAEINVAL;
  }
  if (timeout != nullptr &&
      (timeout->tv_sec < 0 || timeout->tv_usec < 0 ||
       timeout->tv_usec >= 1000000)) {
    return WSAEINVAL;
  }
  if (name == nullptr || name[0] == L'\0') {
    return EAI_NONAME;
  }

  const size_t name_len = wcsnlen_s(name, kAddrInfoAsyncNameCapacity);
  if (name_len >= kAddrInfoAsyncNameCapacity) {
    return WSAEINVAL;
  }
  size_t service_len = 0;
  if (service != nullptr && service[0] != L'\0') {
    service_len = wcsnlen_s(service, kAddrInfoAsyncServiceCapacity);
    if (service_len >= kAddrInfoAsyncServiceCapacity) {
      return WSAEINVAL;
    }
  }
  if (hints != nullptr &&
      (hints->ai_addrlen != 0 || hints->ai_canonname != nullptr ||
       hints->ai_addr != nullptr || hints->ai_blob != nullptr ||
       hints->ai_bloblen != 0 || hints->ai_provider != nullptr ||
       hints->ai_next != nullptr)) {
    return WSANO_RECOVERY;
  }
  if (hints != nullptr && !AddrInfoFlagsSupported(hints->ai_flags)) {
    return WSAEOPNOTSUPP;
  }

  char name_u8[256] = {};
  if (!WideToUtf8(name, name_u8, ARRAYSIZE(name_u8)) ||
      !IsAsciiNameA(name_u8)) {
    return EAI_FAIL;
  }
  const RuntimeProfile* profile = EnvBoxProfile();
  if (profile == nullptr || profile->dns_mode != 1 ||
      DnsUpstreamCount(profile) <= 0) {
    return WSAEOPNOTSUPP;
  }

  EnvBoxAddrInfoAsyncContext* context =
      static_cast<EnvBoxAddrInfoAsyncContext*>(HeapAlloc(
          GetProcessHeap(), HEAP_ZERO_MEMORY,
          sizeof(EnvBoxAddrInfoAsyncContext)));
  if (context == nullptr) return WSA_NOT_ENOUGH_MEMORY;
  context->completion = completion;
  context->overlapped = overlapped;
  context->result = result;
  context->name_space = dw_name_space;
  context->query_deadline = GetTickCount64() + kDnsTotalBudgetMs;
  if (timeout != nullptr) {
    const ULONGLONG requested_ms =
        static_cast<ULONGLONG>(timeout->tv_sec) * 1000ULL +
        (static_cast<ULONGLONG>(timeout->tv_usec) + 999ULL) / 1000ULL;
    const ULONGLONG accepted_at = GetTickCount64();
    const ULONGLONG profile_deadline = accepted_at + kDnsTotalBudgetMs;
    if (requested_ms < kDnsTotalBudgetMs) {
      context->query_deadline = accepted_at + requested_ms;
    } else {
      context->query_deadline = profile_deadline;
    }
  }
  memcpy(context->query_name, name, (name_len + 1) * sizeof(wchar_t));
  if (service_len > 0) {
    memcpy(context->service, service, (service_len + 1) * sizeof(wchar_t));
  }
  if (hints != nullptr) {
    context->hints = *hints;
    context->hints.ai_canonname = nullptr;
    context->hints.ai_addr = nullptr;
    context->hints.ai_blob = nullptr;
    context->hints.ai_provider = nullptr;
    context->hints.ai_next = nullptr;
    context->has_hints = 1;
  }
  context->cancel_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (context->cancel_event == nullptr) {
    HeapFree(GetProcessHeap(), 0, context);
    return WSA_NOT_ENOUGH_MEMORY;
  }
  if (completion == nullptr && overlapped->hEvent != nullptr &&
      !DuplicateHandle(GetCurrentProcess(), overlapped->hEvent,
                       GetCurrentProcess(), &context->notify_event, 0, FALSE,
                       DUPLICATE_SAME_ACCESS)) {
    DWORD error = GetLastError();
    CloseHandle(context->cancel_event);
    HeapFree(GetProcessHeap(), 0, context);
    if (error == ERROR_INVALID_HANDLE) return WSA_INVALID_HANDLE;
    if (error == ERROR_ACCESS_DENIED) return WSAEACCES;
    if (error == ERROR_NOT_ENOUGH_MEMORY || error == ERROR_OUTOFMEMORY)
      return WSA_NOT_ENOUGH_MEMORY;
    return WSASYSCALLFAILURE;
  }
  context->cancel_token = AddrInfoAsyncToken(NextAddrInfoAsyncGeneration());
  if (!RegisterAddrInfoAsync(context)) {
    if (context->notify_event != nullptr) {
      CloseHandle(context->notify_event);
      context->notify_event = nullptr;
    }
    CloseHandle(context->cancel_event);
    HeapFree(GetProcessHeap(), 0, context);
    return WSA_NOT_ENOUGH_MEMORY;
  }
  if (name_handle != nullptr) *name_handle = context->cancel_token;
  // The submission API returns WSA_IO_PENDING, while its result helper reports
  // WSAEINPROGRESS until completion (the native provider uses this state too).
  overlapped->Internal = WSAEINPROGRESS;
  overlapped->InternalHigh = 0;
  overlapped->Pointer = nullptr;
  if (!QueueUserWorkItem(AddrInfoAsyncWorker, context, WT_EXECUTEDEFAULT)) {
    if (name_handle != nullptr && *name_handle == context->cancel_token) {
      *name_handle = nullptr;
    }
    UnregisterAddrInfoAsync(context);
    ReleaseAddrInfoAsyncRef(context);
    return WSA_NOT_ENOUGH_MEMORY;
  }
  EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-virtual-async-worker");
  return WSA_IO_PENDING;
}

static INT WSAAPI HookGetAddrInfoExW(
    PCWSTR name, PCWSTR service, DWORD dw_name_space, LPGUID nlp_id,
    const ADDRINFOEXW* hints, PADDRINFOEXW* result, struct timeval* timeout,
    LPOVERLAPPED overlapped,
    LPLOOKUPSERVICE_COMPLETION_ROUTINE completion, LPHANDLE name_handle) {
  DWORD err = GetLastError();
  const ULONGLONG accepted_at = GetTickCount64();
  ULONGLONG query_deadline = accepted_at + kDnsTotalBudgetMs;
  const int asynchronous = overlapped != nullptr || completion != nullptr ||
                           name_handle != nullptr;
  if (g_view_active && asynchronous) {
    INT status = StartGetAddrInfoExWAsync(
        name, service, dw_name_space, nlp_id, hints, result, timeout,
        overlapped, completion, name_handle);
    SetLastError(err);
    return status;
  }
  if (g_view_active &&
      (nlp_id != nullptr ||
       (dw_name_space != 0 && dw_name_space != NS_ALL &&
        dw_name_space != NS_DNS))) {
    if (result != nullptr) *result = nullptr;
    EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-unsupported-provider");
    SetLastError(err);
    return WSAEOPNOTSUPP;
  }
  if (g_view_active && timeout != nullptr) {
    if (timeout->tv_sec < 0 || timeout->tv_usec < 0 || timeout->tv_usec >= 1000000) {
      if (result != nullptr) *result = nullptr;
      SetLastError(err);
      return WSAEINVAL;
    }
    const ULONGLONG requested_ms = static_cast<ULONGLONG>(timeout->tv_sec) * 1000 +
        (static_cast<ULONGLONG>(timeout->tv_usec) + 999) / 1000;
    if (requested_ms < kDnsTotalBudgetMs) query_deadline = accepted_at + requested_ms;
  }
  if (overlapped != nullptr || completion != nullptr) {
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "fail-open-async");
    SetLastError(err);
    return r;
  }
  if (dw_name_space != 0 && dw_name_space != NS_ALL && dw_name_space != NS_DNS) {
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "fail-open");
    SetLastError(err);
    return r;
  }

  if (!g_view_active) {
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "dns-host");
    SetLastError(err);
    return r;
  }

  int flags = hints ? hints->ai_flags : 0;
  int family = hints ? hints->ai_family : AF_UNSPEC;
  int stype = hints ? hints->ai_socktype : 0;
  int proto = hints ? hints->ai_protocol : 0;
  if (!AddrInfoFlagsSupported(flags)) {
    EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-unsupported-flags");
    SetLastError(err);
    return WSAEOPNOTSUPP;
  }

  if (name == nullptr || name[0] == L'\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeW(name) || IsLocalMachineDnsNameW(name)) {
    LocalhostResolutionScope local(name);
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "numeric-or-passthrough");
    SetLastError(err);
    return r;
  }

  char name_u8[256];
  if (!WideToUtf8(name, name_u8, (int)sizeof(name_u8)) ||
      !IsAsciiNameA(name_u8)) {
    EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-unsupported-name");
    SetLastError(err);
    return EAI_FAIL;
  }
  char svc_u8[64];
  svc_u8[0] = '\0';
  if (service != nullptr && service[0] != L'\0') {
    if (!WideToUtf8(service, svc_u8, (int)sizeof(svc_u8))) {
      EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-unsupported-service");
      SetLastError(err);
      return EAI_SERVICE;
    }
  }

  DnsAddrs addrs;
  unsigned short port = 0;
  int rc = ResolveRoutedA(name_u8, svc_u8, family, stype, proto, flags, &addrs,
                          &port, query_deadline);
  if (rc == EAI_AGAIN) {
    EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-profile-error");
    SetLastError(err);
    return EAI_AGAIN;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    const char* kind = addrs.nodata ? "nodata" : "nxdomain";
    _snprintf_s(note, sizeof(note), _TRUNCATE, "%s node=%.64s", kind,
                name_u8);
    EnvBoxAuditEvent("GetAddrInfoExW", 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-profile-error");
    SetLastError(err);
    return rc;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOEXW chain =
      BuildChain<ADDRINFOEXW, wchar_t, decltype(&AllocAddrInfoExW)>(
          pairs, np, port, family, stype, proto, flags, name,
          &AllocAddrInfoExW);
  if (chain == nullptr) {
    EnvBoxAuditEvent("GetAddrInfoExW", 1, "dns-profile-error");
    SetLastError(err);
    return EAI_MEMORY;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoExWChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route n=%d", np);
  EnvBoxAuditEvent("GetAddrInfoExW", 1, note);
  SetLastError(err);
  return 0;
}

// ---------------------------------------------------------------------------
// DnsQueryEx and DnsQuery_A / W / UTF8 (all record types).
// ---------------------------------------------------------------------------

static const DWORD kDnsSupportedOptions = DNS_QUERY_USE_TCP_ONLY | DNS_QUERY_NO_RECURSION |
      DNS_QUERY_BYPASS_CACHE | DNS_QUERY_NO_LOCAL_NAME | DNS_QUERY_NO_HOSTS_FILE |
      DNS_QUERY_NO_NETBT | DNS_QUERY_WIRE_ONLY | DNS_QUERY_NO_MULTICAST |
      DNS_QUERY_TREAT_AS_FQDN | DNS_QUERY_DONT_RESET_TTL_VALUES |
      DNS_QUERY_DISABLE_IDN_ENCODING;

enum DnsQueryFlavor {
  kDnsFlavorA = 0,
  kDnsFlavorW = 1,
  kDnsFlavorUtf8 = 2,
};

static DNS_STATUS RouteDnsQuery(const char* name_u8, WORD wtype, DWORD opts,
                                PDNS_RECORD* out, PVOID* reserved,
                                DnsQueryFlavor flavor, const wchar_t* name_w,
                                const char* api, HANDLE cancel_event = nullptr,
                                ULONGLONG accepted_deadline = 0);

// DnsQueryEx's native asynchronous provider can ignore pDnsServerList on
// machines with DNS Client policy/NRPT.  For VirtualView, own the complete
// async operation and reuse the bounded Profile wire route instead.  The
// caller's result/cancel storage remains valid until the callback, as required
// by DnsQueryEx; this state only borrows those pointers and owns its event.
struct EnvBoxDnsAsyncContext {
  PDNS_QUERY_COMPLETION_ROUTINE callback;
  PVOID user_context;
  PDNS_QUERY_RESULT results;
  PDNS_QUERY_CANCEL cancel_handle;
  HANDLE cancel_event;
  WORD query_type;
  DWORD query_options;
  ULONGLONG query_deadline;
  ULONGLONG cancel_generation;
  unsigned char original_cancel[sizeof(DNS_QUERY_CANCEL)];
  int cancel_token_written;
  int completed;
  wchar_t query_name[256];
  char name_u8[256];
  volatile LONG cancel_requested;
  int registered;
  EnvBoxDnsAsyncContext* next;
};

static const LONG kDnsAsyncMaxPending = 64;
static const ULONGLONG kDnsAsyncCancelMagic = 0x454E56424F584451ULL;
static_assert(sizeof(DNS_QUERY_CANCEL) >= 16,
              "DNS_QUERY_CANCEL must provide opaque token storage");
static volatile LONG64 g_dns_async_generation = 0;
static LONG g_dns_async_work_count = 0;
static LONG g_dns_async_pending_count = 0;
static EnvBoxDnsAsyncContext* g_dns_async_pending = nullptr;
static SRWLOCK g_dns_async_lock = SRWLOCK_INIT;

static int DnsAsyncCancelled(const EnvBoxDnsAsyncContext* context) {
  return context != nullptr &&
         InterlockedCompareExchange(
             const_cast<volatile LONG*>(&context->cancel_requested), 0, 0) !=
             0;
}

// The pending map is only for finding EnvBox-owned cancel handles.  It never
// runs a caller callback while held.  The worker removes its state after the
// callback returns, so a concurrent cancel can safely signal the event without
// taking ownership or freeing the context.
static EnvBoxDnsAsyncContext* FindDnsAsyncLocked(ULONGLONG generation);

static ULONGLONG NextDnsAsyncGeneration() {
  LONG64 generation = InterlockedIncrement64(&g_dns_async_generation);
  if (generation <= 0) {
    InterlockedExchange64(&g_dns_async_generation, 1);
    generation = 1;
  }
  return (ULONGLONG)generation;
}

// All token access is serialized with publication, completion and cancellation
// under g_dns_async_lock; callers may cancel concurrently with callback reentry.
static int ReadDnsAsyncToken(PDNS_QUERY_CANCEL cancel_handle,
                             ULONGLONG* generation) {
  if (cancel_handle == nullptr || generation == nullptr) {
    return 0;
  }
  ULONGLONG magic = 0;
  ULONGLONG value = 0;
  memcpy(&magic, cancel_handle, sizeof(magic));
  memcpy(&value, reinterpret_cast<const unsigned char*>(cancel_handle) +
                         sizeof(magic),
         sizeof(value));
  if (magic != kDnsAsyncCancelMagic) {
    return 0;
  }
  *generation = value;
  return 1;
}

static void WriteDnsAsyncToken(PDNS_QUERY_CANCEL cancel_handle,
                               ULONGLONG generation) {
  unsigned char token[sizeof(DNS_QUERY_CANCEL)] = {};
  memcpy(token, &kDnsAsyncCancelMagic, sizeof(kDnsAsyncCancelMagic));
  memcpy(token + sizeof(kDnsAsyncCancelMagic), &generation,
         sizeof(generation));
  memcpy(cancel_handle, token, sizeof(token));
}

static void RestoreDnsAsyncCancelToken(EnvBoxDnsAsyncContext* context) {
  if (context == nullptr) {
    return;
  }
  // The caller may reuse the same DNS_QUERY_CANCEL storage from inside the
  // completion callback.  Restore only while our exact generation is still
  // published; otherwise a reentrant request has already installed a newer
  // token and the old context must leave it untouched.
  AcquireSRWLockExclusive(&g_dns_async_lock);
  if (context->cancel_handle != nullptr && context->cancel_token_written) {
    ULONGLONG generation = 0;
    if (ReadDnsAsyncToken(context->cancel_handle, &generation) &&
        generation == context->cancel_generation) {
      memcpy(context->cancel_handle, context->original_cancel,
             sizeof(context->original_cancel));
      context->cancel_token_written = 0;
    }
  }
  ReleaseSRWLockExclusive(&g_dns_async_lock);
}

static int RegisterDnsAsync(EnvBoxDnsAsyncContext* context) {
  if (context == nullptr) {
    return 0;
  }
  int ok = 0;
  AcquireSRWLockExclusive(&g_dns_async_lock);
  if (g_dns_async_work_count < kDnsAsyncMaxPending) {
    g_dns_async_work_count++;
    context->registered = 1;
    if (context->cancel_handle != nullptr) {
      context->next = g_dns_async_pending;
      g_dns_async_pending = context;
      g_dns_async_pending_count++;
    }
    ok = 1;
  }
  ReleaseSRWLockExclusive(&g_dns_async_lock);
  return ok;
}

static EnvBoxDnsAsyncContext* FindDnsAsyncLocked(
    ULONGLONG generation) {
  for (EnvBoxDnsAsyncContext* p = g_dns_async_pending; p != nullptr;
       p = p->next) {
    if (p->cancel_generation == generation) {
      return p;
    }
  }
  return nullptr;
}

static int UnregisterDnsAsync(EnvBoxDnsAsyncContext* context) {
  if (context == nullptr || !context->registered) {
    return 0;
  }
  int was_registered = 0;
  AcquireSRWLockExclusive(&g_dns_async_lock);
  if (context->registered) {
    if (context->cancel_handle != nullptr) {
      EnvBoxDnsAsyncContext** link = &g_dns_async_pending;
      while (*link != nullptr) {
        if (*link == context) {
          *link = context->next;
          if (g_dns_async_pending_count > 0) {
            g_dns_async_pending_count--;
          }
          break;
        }
        link = &(*link)->next;
      }
    }
    context->registered = 0;
    was_registered = 1;
  }
  ReleaseSRWLockExclusive(&g_dns_async_lock);
  return was_registered;
}

static void ReleaseDnsAsyncWork() {
  AcquireSRWLockExclusive(&g_dns_async_lock);
  if (g_dns_async_work_count > 0) {
    g_dns_async_work_count--;
  }
  ReleaseSRWLockExclusive(&g_dns_async_lock);
}

static void FreeDnsAsync(EnvBoxDnsAsyncContext* context) {
  if (context == nullptr) {
    return;
  }
  int was_registered = UnregisterDnsAsync(context);
  if (context->cancel_event != nullptr) {
    CloseHandle(context->cancel_event);
    context->cancel_event = nullptr;
  }
  // Keep the bounded-work slot occupied until the callback has returned and
  // the worker's event resource has been released. This prevents a callback
  // re-entry from briefly exceeding the cap while the old worker is unwinding.
  if (was_registered) {
    ReleaseDnsAsyncWork();
  }
  HeapFree(GetProcessHeap(), 0, context);
}

static void FreeDnsAsyncRecords(PDNS_RECORD records) {
  if (records == nullptr) {
    return;
  }
  TrueDnsFree(records, DnsFreeRecordList);
}

static void CompleteDnsAsync(EnvBoxDnsAsyncContext* context, DNS_STATUS status,
                             PDNS_RECORD records) {
  if (context == nullptr) {
    return;
  }
  AcquireSRWLockExclusive(&g_dns_async_lock);
  const int cancelled = DnsAsyncCancelled(context);
  context->completed = 1;
  // Keep the opaque token as a local stale-token tombstone. Restoring original
  // bytes before callback would send a callback-side cancel to the Windows
  // provider; restoring after callback could overwrite a reentrant request or
  // access storage the callback freed. This context relinquishes write ownership
  // before notifying and never accesses that borrowed storage again.
  context->cancel_token_written = 0;
  ReleaseSRWLockExclusive(&g_dns_async_lock);
  if (cancelled) {
    if (records != nullptr) {
      FreeDnsAsyncRecords(records);
      records = nullptr;
    }
    status = ERROR_CANCELLED;
  }
  if (context->results != nullptr) {
    context->results->QueryOptions = context->query_options;
    context->results->pQueryRecords = records;
    context->results->Reserved = nullptr;
    InterlockedExchange(
        reinterpret_cast<volatile LONG*>(&context->results->QueryStatus),
        static_cast<LONG>(status));
  }
  // Keep the context registered while caller code runs. A callback may reuse
  // the same DNS_QUERY_CANCEL storage for another request; that request gets
  // a new generation token and remains independently discoverable. The worker
  // owns the old context until the callback returns, then releases the map
  // entry and decrements the bounded work count.
  if (context->callback != nullptr) {
    context->callback(context->user_context, context->results);
  }
  FreeDnsAsync(context);
}

static DWORD WINAPI DnsAsyncWorker(PVOID parameter) {
  EnvBoxDnsAsyncContext* context =
      static_cast<EnvBoxDnsAsyncContext*>(parameter);
  if (context == nullptr) {
    return 0;
  }

  PDNS_RECORD records = nullptr;
  DNS_STATUS status = RouteDnsQuery(
      context->name_u8, context->query_type, context->query_options, &records,
      nullptr, kDnsFlavorW, context->query_name, "DnsQueryEx",
      context->cancel_event, context->query_deadline);
  CompleteDnsAsync(context, status, records);
  return 0;
}

static DNS_STATUS WINAPI HookDnsCancelQuery(PDNS_QUERY_CANCEL cancel_handle) {
  DWORD err = GetLastError();
  AcquireSRWLockExclusive(&g_dns_async_lock);
  if (cancel_handle != nullptr) {
    ULONGLONG generation = 0;
    if (ReadDnsAsyncToken(cancel_handle, &generation)) {
      // An EnvBox token is always handled locally, even after its worker has
      // completed. In particular, never pass a copied/stale token to the
      // native API where it could be interpreted as an unrelated provider
      // handle.
      EnvBoxDnsAsyncContext* context =
          generation == 0 ? nullptr : FindDnsAsyncLocked(generation);
      if (context != nullptr && !context->completed) {
        InterlockedExchange(&context->cancel_requested, 1);
        BOOL signaled = context->cancel_event == nullptr ||
                        SetEvent(context->cancel_event);
        ReleaseSRWLockExclusive(&g_dns_async_lock);
        SetLastError(err);
        EnvBoxAuditEvent("DnsCancelQuery", signaled ? 1 : 0,
                         signaled ? "dns-virtual-async-cancel"
                                  : "fail-open-async-cancel-signal");
        return signaled ? ERROR_SUCCESS : ERROR_GEN_FAILURE;
      }
      ReleaseSRWLockExclusive(&g_dns_async_lock);
      SetLastError(err);
      EnvBoxAuditEvent("DnsCancelQuery", 0, "dns-virtual-async-stale");
      return ERROR_INVALID_PARAMETER;
    }
  }
  ReleaseSRWLockExclusive(&g_dns_async_lock);
  DNS_STATUS status = TrueDnsCancelQuery(cancel_handle);
  EnvBoxAuditEvent("DnsCancelQuery", 0, "dns-host-or-unowned");
  SetLastError(err);
  return status;
}

static int DnsNameEqualsW(const wchar_t* query, const wchar_t* local) {
  if (query == nullptr || local == nullptr) return 0;
  size_t query_len = wcslen(query);
  size_t local_len = wcslen(local);
  if (query_len > 0 && query[query_len - 1] == L'.') query_len--;
  if (local_len > 0 && local[local_len - 1] == L'.') local_len--;
  return query_len == local_len &&
         _wcsnicmp(query, local, query_len) == 0;
}

// Windows completes local-machine A/AAAA DnsQueryEx calls synchronously and
// does not invoke a supplied callback. Preserve that API shape for local names.
static int IsLocalMachineDnsNameW(const wchar_t* name) {
  if (DnsNameEqualsW(name, L"localhost")) return 1;
  // A configured name is an information view, not a host-resolver alias.
  // Both that label and the real host name must follow Profile DNS routing.
  if (EnvBoxProfile()->identity_computer_name[0]) return 0;
  wchar_t local[256] = {};
  DWORD cap = ARRAYSIZE(local);
  if (GetComputerNameW(local, &cap) && DnsNameEqualsW(name, local)) return 1;
  cap = ARRAYSIZE(local);
  if (GetComputerNameExW(ComputerNameDnsHostname, local, &cap) &&
      DnsNameEqualsW(name, local)) {
    return 1;
  }
  cap = ARRAYSIZE(local);
  return GetComputerNameExW(ComputerNameDnsFullyQualified, local, &cap) &&
         DnsNameEqualsW(name, local);
}

static int IsLocalDnsQueryW(const wchar_t* name, WORD type, DWORD options) {
  if (type != DNS_TYPE_A && type != DNS_TYPE_AAAA) return 0;
  in_addr v4;
  in6_addr v6;
  if (name && ((type == DNS_TYPE_A && InetPtonW(AF_INET, name, &v4) == 1) ||
               (type == DNS_TYPE_AAAA && InetPtonW(AF_INET6, name, &v6) == 1))) return 1;
  return !(options & (DNS_QUERY_WIRE_ONLY | DNS_QUERY_NO_LOCAL_NAME)) &&
         IsLocalMachineDnsNameW(name);
}

static DNS_STATUS WINAPI HookDnsQueryEx(PDNS_QUERY_REQUEST request,
                                        PDNS_QUERY_RESULT results,
                                        PDNS_QUERY_CANCEL cancel) {
  DWORD err = GetLastError();
  ULONGLONG query_deadline = GetTickCount64() + kDnsTotalBudgetMs;
  if (!g_view_active) {
    DNS_STATUS st = TrueDnsQueryEx(request, results, cancel);
    EnvBoxAuditEvent("DnsQueryEx", 0,
                     "dns-host");
    SetLastError(err);
    return st;
  }
  if (request == nullptr || results == nullptr) {
    EnvBoxAuditEvent("DnsQueryEx", 1, "dns-invalid-input");
    SetLastError(err);
    return ERROR_INVALID_PARAMETER;
  }

  // This invocation belongs to Windows' synchronous localhost resolver.
  // Its private request/result layout must be interpreted by Windows itself,
  // before our public v1 validator, without extending Profile wire support.
  if (g_localhost_resolution_depth != 0 &&
      request->Version != DNS_QUERY_REQUEST_VERSION1) {
    DNS_STATUS st = TrueDnsQueryEx(request, results, cancel);
    EnvBoxAuditEvent("DnsQueryEx", 0, "dns-localhost-native-reentry");
    SetLastError(err);
    return st;
  }

  // Only the v1 request/result layouts are handled. Later SDK versions may
  // append fields whose semantics this hook must not silently truncate.
  if (request->Version != DNS_QUERY_REQUEST_VERSION1) {
    DNS_STATUS st = ERROR_NOT_SUPPORTED;
    EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-unsupported-version");
    SetLastError(err);
    return st;
  }
  if (results->Version != DNS_QUERY_RESULTS_VERSION1 ||
      results->QueryStatus != ERROR_SUCCESS ||
      results->QueryOptions != DNS_QUERY_STANDARD ||
      results->pQueryRecords != nullptr || results->Reserved != nullptr) {
    DNS_STATUS st = ERROR_INVALID_PARAMETER;
    EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-invalid-result");
    SetLastError(err);
    return st;
  }

  // The bounded wire client implements a subset of query options. Reject
  // every input whose semantics would otherwise be lost, especially the
  // high query-option bits and caller-selected server/interface paths.
  if ((request->QueryOptions & ~static_cast<ULONG64>(kDnsSupportedOptions)) != 0 ||
      request->pDnsServerList != nullptr || request->InterfaceIndex != 0 ||
      (request->pQueryCompletionCallback == nullptr && cancel != nullptr) ||
      request->QueryType == 0) {
    DNS_STATUS st = ERROR_NOT_SUPPORTED;
    EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-unsupported-input");
    SetLastError(err);
    return st;
  }

  if ((request->QueryName == nullptr &&
       (request->QueryType == DNS_TYPE_A || request->QueryType == DNS_TYPE_AAAA) &&
       !(request->QueryOptions & (DNS_QUERY_WIRE_ONLY | DNS_QUERY_NO_LOCAL_NAME))) ||
      IsLocalDnsQueryW(request->QueryName, request->QueryType, (DWORD)request->QueryOptions)) {
    DNS_STATUS st = TrueDnsQueryEx(request, results, cancel);
    EnvBoxAuditEvent("DnsQueryEx", 0, "local-machine-passthrough");
    SetLastError(err);
    return st;
  }

  char name_u8[256];
  if (!WideToUtf8(request->QueryName, name_u8, (int)sizeof(name_u8))) {
    DNS_STATUS st = ERROR_INVALID_PARAMETER;
    EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-name-conversion");
    SetLastError(err);
    return st;
  }

  if (request->pQueryCompletionCallback != nullptr) {
    if (cancel != nullptr && !g_dns_cancel_hook_attached) {
      DNS_STATUS st = ERROR_NOT_SUPPORTED;
      EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-async-cancel-hook");
      SetLastError(err);
      return st;
    }
    const RuntimeProfile* profile = EnvBoxProfile();
    if (profile == nullptr || profile->dns_mode != 1 ||
        DnsUpstreamCount(profile) <= 0 ||
        !IsAsciiNameA(name_u8)) {
      DNS_STATUS st = ERROR_NOT_SUPPORTED;
      EnvBoxAuditEvent("DnsQueryEx", 0,
                       profile == nullptr || profile->dns_mode != 1
                           ? "dns-rejected-async-no-profile-dns"
                           : "dns-rejected-async-unsupported-name");
      SetLastError(err);
      return st;
    }

    size_t name_len = wcslen(request->QueryName);
    if (name_len >= 256) {
      DNS_STATUS st = ERROR_INVALID_PARAMETER;
      EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-async-name-too-long");
      SetLastError(err);
      return st;
    }

    EnvBoxDnsAsyncContext* context =
        static_cast<EnvBoxDnsAsyncContext*>(HeapAlloc(
            GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof(EnvBoxDnsAsyncContext)));
    if (context == nullptr) {
      DNS_STATUS st = ERROR_NOT_ENOUGH_MEMORY;
      EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-async-allocation");
      SetLastError(err);
      return st;
    }
    context->callback = request->pQueryCompletionCallback;
    context->user_context = request->pQueryContext;
    context->results = results;
    context->cancel_handle = cancel;
    context->query_type = request->QueryType;
    context->query_options = (DWORD)request->QueryOptions;
    context->query_deadline = query_deadline;
    if (cancel != nullptr) {
      AcquireSRWLockExclusive(&g_dns_async_lock);
      memcpy(context->original_cancel, cancel,
             sizeof(context->original_cancel));
      ReleaseSRWLockExclusive(&g_dns_async_lock);
      context->cancel_generation = NextDnsAsyncGeneration();
    }
    memcpy(context->query_name, request->QueryName,
           (name_len + 1) * sizeof(wchar_t));
    memcpy(context->name_u8, name_u8, sizeof(context->name_u8));

    const char* async_fallback = nullptr;
    if (cancel != nullptr) {
      context->cancel_event =
          CreateEventW(nullptr, TRUE, FALSE, nullptr);
      if (context->cancel_event == nullptr) {
        async_fallback = "dns-rejected-async-cancel-event";
      }
    }
    if (async_fallback == nullptr && !RegisterDnsAsync(context)) {
      async_fallback = "dns-rejected-async-pending-limit";
    }
    if (async_fallback != nullptr) {
      RestoreDnsAsyncCancelToken(context);
      FreeDnsAsync(context);
      DNS_STATUS st = ERROR_NOT_ENOUGH_MEMORY;
      EnvBoxAuditEvent("DnsQueryEx", 1, async_fallback);
      SetLastError(err);
      return st;
    }

    // DnsQueryEx exposes this field while the callback is pending. Publish it
    // before queueing so a fast worker cannot race a later pending write.
    InterlockedExchange(
        reinterpret_cast<volatile LONG*>(&results->QueryStatus),
        DNS_REQUEST_PENDING);
    if (cancel != nullptr) {
      AcquireSRWLockExclusive(&g_dns_async_lock);
      WriteDnsAsyncToken(cancel, context->cancel_generation);
      context->cancel_token_written = 1;
      ReleaseSRWLockExclusive(&g_dns_async_lock);
    }
    if (!QueueUserWorkItem(DnsAsyncWorker, context, WT_EXECUTEDEFAULT)) {
      InterlockedExchange(
          reinterpret_cast<volatile LONG*>(&results->QueryStatus),
          ERROR_SUCCESS);
      RestoreDnsAsyncCancelToken(context);
      FreeDnsAsync(context);
      DNS_STATUS st = ERROR_NOT_ENOUGH_MEMORY;
      EnvBoxAuditEvent("DnsQueryEx", 1, "dns-rejected-async-queue");
      SetLastError(err);
      return st;
    }

    EnvBoxAuditEvent("DnsQueryEx", 1, "dns-virtual-async-worker");
    SetLastError(err);
    return DNS_REQUEST_PENDING;
  }

  PDNS_RECORD records = nullptr;
  DNS_STATUS st = RouteDnsQuery(name_u8, request->QueryType,
                                (DWORD)request->QueryOptions, &records, nullptr,
                                kDnsFlavorW, request->QueryName, "DnsQueryEx", nullptr,
                                query_deadline);
  results->QueryStatus = st;
  results->QueryOptions = request->QueryOptions;
  results->pQueryRecords = records;
  results->Reserved = nullptr;
  SetLastError(err);
  return st;
}

// Matching native API used only for Host mode and purely local lookups.

static DNS_STATUS CallTrueDnsQuery(DnsQueryFlavor flavor, const char* name_u8,
                                   const wchar_t* name_w, WORD wtype, DWORD opts,
                                   PDNS_RECORD* out, PVOID* reserved) {
  switch (flavor) {
    case kDnsFlavorW:
      return TrueDnsQuery_W(name_w, wtype, opts, nullptr, out, reserved);
    case kDnsFlavorUtf8:
      return TrueDnsQuery_UTF8(name_u8, wtype, opts, nullptr, out, reserved);
    case kDnsFlavorA:
    default:
      return TrueDnsQuery_A(name_u8, wtype, opts, nullptr, out, reserved);
  }
}

static DNS_STATUS RouteDnsQuery(const char* name_u8, WORD wtype, DWORD opts,
                                PDNS_RECORD* out, PVOID* reserved,
                                DnsQueryFlavor flavor, const wchar_t* name_w,
                                const char* api, HANDLE cancel_event,
                                ULONGLONG accepted_deadline) {
  DWORD err = GetLastError();
  ULONGLONG deadline = accepted_deadline ? accepted_deadline : GetTickCount64() + kDnsTotalBudgetMs;
  if (out != nullptr) {
    *out = nullptr;
  }
  if (!g_view_active) {
    DNS_STATUS st =
        CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    EnvBoxAuditEvent(api, 0, "dns-host");
    SetLastError(err);
    return st;
  }

  if (opts & ~kDnsSupportedOptions) {
    EnvBoxAuditEvent(api, 1, "dns-unsupported-options");
    SetLastError(err);
    return ERROR_NOT_SUPPORTED;
  }
  if (name_u8 == nullptr || name_u8[0] == '\0' || !IsAsciiNameA(name_u8)) {
    EnvBoxAuditEvent(api, 1, "dns-unsupported-name");
    SetLastError(err);
    return ERROR_INVALID_NAME;
  }
  wchar_t local_name[256] = {};
  if (MultiByteToWideChar(CP_UTF8, 0, name_u8, -1, local_name, ARRAYSIZE(local_name)) &&
      IsLocalDnsQueryW(local_name, wtype, opts)) {
    DNS_STATUS st = CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    SetLastError(err);
    return st;
  }
  unsigned char encoded[256];
  if (EncodeDnsName(name_u8, encoded, sizeof(encoded)) <= 0 || wtype == 0 || out == nullptr) {
    SetLastError(err);
    return ERROR_INVALID_PARAMETER;
  }
  DnsWinsockScope winsock;
  DNS_STATUS status = winsock.active ? ERROR_TIMEOUT : WSANOTINITIALISED;
  PDNS_RECORD records = nullptr;
  const RuntimeProfile* profile = EnvBoxProfile();
  char current[256];
  strcpy_s(current, name_u8);
  PDNS_RECORD tail = nullptr;
  for (int hop = 0; hop <= kDnsMaxCnameHops && winsock.active && profile && profile->dns_mode == 1; ++hop) {
    PDNS_RECORD step = nullptr;
    int count = DnsUpstreamCount(profile);
    for (int i = 0; i < count; ++i) {
      if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) {
        status = ERROR_CANCELLED;
        break;
      }
      ULONGLONG now = GetTickCount64();
      if (now >= deadline) break;
      DnsTransportEndpoint endpoint;
      if (!DnsEndpoint(profile, i, &endpoint)) continue;
      DNS_STATUS response = ERROR_TIMEOUT;
      int result = DnsQueryOne(endpoint, current, wtype, DnsAttemptDeadline(deadline),
                              cancel_event, nullptr, &step, &response, opts);
      if (result < 0) { status = ERROR_CANCELLED; break; }
      if (result == 0) continue;
      status = response;
      if (status == ERROR_SUCCESS || status == DNS_INFO_NO_RECORDS ||
          status == DNS_ERROR_RCODE_NAME_ERROR) break;
      if (step) { TrueDnsFree(step, DnsFreeRecordList); step = nullptr; }
    }
    if (status != ERROR_SUCCESS) {
      if (step) TrueDnsFree(step, DnsFreeRecordList);
      break;
    }
    int has_answer = 0;
    wchar_t* alias = nullptr;
    for (PDNS_RECORD rr = step; rr; rr = rr->pNext) {
      if (rr->Flags.S.Section != DnsSectionAnswer) continue;
      if (rr->wType == wtype || wtype == DNS_TYPE_ALL) has_answer = 1;
      if (rr->wType == DNS_TYPE_CNAME)
        alias = reinterpret_cast<wchar_t*>(rr->Data.PTR.pNameHost);
    }
    if (!has_answer && !alias) {
      if (step) TrueDnsFree(step, DnsFreeRecordList);
      status = DNS_INFO_NO_RECORDS;
      break;
    }
    char target[256] = {};
    if (!has_answer && (!WideToUtf8(alias, target, sizeof(target)) ||
        !IsAsciiNameA(target) || hop == kDnsMaxCnameHops)) {
      TrueDnsFree(step, DnsFreeRecordList);
      status = DNS_ERROR_CNAME_LOOP;
      break;
    }
    if (tail) tail->pNext = step;
    else records = step;
    tail = step;
    while (tail && tail->pNext) tail = tail->pNext;
    if (has_answer) break;
    strcpy_s(current, target);
    status = ERROR_TIMEOUT;
  }
  if (records && status == ERROR_SUCCESS && flavor != kDnsFlavorW) {
    PDNS_RECORD converted = DnsRecordSetCopyEx(records, DnsCharSetUnicode,
        flavor == kDnsFlavorUtf8 ? DnsCharSetUtf8 : DnsCharSetAnsi);
    TrueDnsFree(records, DnsFreeRecordList);
    records = converted;
    if (!records) status = ERROR_NOT_ENOUGH_MEMORY;
  }
  if (status != ERROR_SUCCESS && records) {
    TrueDnsFree(records, DnsFreeRecordList);
    records = nullptr;
  }
  *out = records;
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route type=%u status=%ld",
              (unsigned)wtype, status);
  EnvBoxAuditEvent(api, 1, note);
  SetLastError(err);
  return status;
}

static DNS_STATUS WINAPI HookDnsQuery_A(PCSTR name, WORD wtype, DWORD opts,
                                        PVOID extra, PDNS_RECORD* out,
                                        PVOID* reserved) {
  (void)extra;
  return RouteDnsQuery(name, wtype, opts, out, reserved, kDnsFlavorA, nullptr,
                       "DnsQuery_A");
}

static DNS_STATUS WINAPI HookDnsQuery_UTF8(PCSTR name, WORD wtype, DWORD opts,
                                           PVOID extra, PDNS_RECORD* out,
                                           PVOID* reserved) {
  (void)extra;
  return RouteDnsQuery(name, wtype, opts, out, reserved, kDnsFlavorUtf8,
                       nullptr, "DnsQuery_UTF8");
}

static DNS_STATUS WINAPI HookDnsQuery_W(PCWSTR name, WORD wtype, DWORD opts,
                                        PVOID extra, PDNS_RECORD* out,
                                        PVOID* reserved) {
  (void)extra;
  DWORD err = GetLastError();
  char name_u8[256];
  if (name == nullptr || !WideToUtf8(name, name_u8, (int)sizeof(name_u8))) {
    DNS_STATUS st = g_view_active ? ERROR_INVALID_NAME :
        TrueDnsQuery_W(name, wtype, opts, nullptr, out, reserved);
    EnvBoxAuditEvent("DnsQuery_W", g_view_active, "invalid-name");
    SetLastError(err);
    return st;
  }
  DNS_STATUS st = RouteDnsQuery(name_u8, wtype, opts, out, reserved,
                                kDnsFlavorW, name, "DnsQuery_W");
  SetLastError(err);
  return st;
}

static DNS_STATUS WINAPI HookDnsQueryRaw(void* request, void* cancel) {
  if (!g_view_active) return TrueDnsQueryRaw(request, cancel);
  DWORD error = GetLastError();
  EnvBoxAuditEvent("DnsQueryRaw", 1, "dns-unsupported-raw");
  SetLastError(error);
  return ERROR_NOT_SUPPORTED;
}

static int g_identity_adapter_ready = 0;
int EnvBoxAdapterAddressIdentityReady() { return g_identity_adapter_ready; }
int EnvBoxDnsHooksReady() { return g_dns_hooks_ready; }

int EnvBoxInstallDnsHooks() {
  BuildDnsView();
  g_dns_hooks_ready = 0;
  int ok = 0;
  const int network_params = EnvBoxAttach(&TrueGetNetworkParams, HookGetNetworkParams);
  const int adapters = EnvBoxAttach(&TrueGetAdaptersAddresses, HookGetAdaptersAddresses);
  g_identity_adapter_ready = adapters;
  ok += network_params + adapters;

  // DNS routing only under VirtualView + Profile servers (Host path untouched).
  if (!g_view_active) {
    g_dns_hooks_ready = network_params && adapters;
    EnvBoxAuditEvent("DnsQueryEx", 0, "dns-host");
    return ok;
  }

  // Never return an OwnedAlloc chain unless the corresponding public free API
  // is hooked too. Partial Detours attach failure must fail open as a pair.
  int free_a = EnvBoxAttach(&Truefreeaddrinfo, Hookfreeaddrinfo);
  int free_w = EnvBoxAttach(&TrueFreeAddrInfoW, HookFreeAddrInfoW);
  int free_ex_a = EnvBoxAttach(&TrueFreeAddrInfoExA, HookFreeAddrInfoExA);
  int free_ex_w = EnvBoxAttach(&TrueFreeAddrInfoExW, HookFreeAddrInfoExW);
  ok += free_a + free_w + free_ex_a + free_ex_w;
  const int resolver_a = free_a && EnvBoxAttach(&Truegetaddrinfo, Hookgetaddrinfo);
  const int resolver_w = free_w && EnvBoxAttach(&TrueGetAddrInfoW, HookGetAddrInfoW);
  const int resolver_ex_a = free_ex_a && EnvBoxAttach(&TrueGetAddrInfoExA, HookGetAddrInfoExA);
  const int resolver_ex_w = free_ex_w && EnvBoxAttach(&TrueGetAddrInfoExW, HookGetAddrInfoExW);
  const int resolver_ex_cancel = EnvBoxAttach(&TrueGetAddrInfoExCancel,
                                              HookGetAddrInfoExCancel);
  // Windows implements GetAddrInfoExOverlappedResult as the documented
  // OVERLAPPED status read.  On current x64 ws2_32 its exported prologue is
  // intentionally too short for a Detours trampoline; our worker publishes
  // Internal/InternalHigh/Pointer before signaling, so the native helper is
  // the correct ABI path and remains available without a detour.
  const int resolver_ex_overlapped = 1;
  // Report every installed detour. The status-only result helper is native,
  // while the newly attached async cancel hook contributes to the actual count.
  ok += resolver_a + resolver_w + resolver_ex_a + resolver_ex_w + resolver_ex_cancel;
  // Native dnsapi records can always be released by the original DnsFree.
  const int query_a = EnvBoxAttach(&TrueDnsQuery_A, HookDnsQuery_A);
  const int query_w = EnvBoxAttach(&TrueDnsQuery_W, HookDnsQuery_W);
  const int query_utf8 = EnvBoxAttach(&TrueDnsQuery_UTF8, HookDnsQuery_UTF8);
  ok += query_a + query_w + query_utf8;
  g_dns_cancel_hook_attached =
      EnvBoxAttach(&TrueDnsCancelQuery, HookDnsCancelQuery);
  ok += g_dns_cancel_hook_attached;
  const int query_ex = EnvBoxAttach(&TrueDnsQueryEx, HookDnsQueryEx);
  ok += query_ex;
  HMODULE dnsapi = GetModuleHandleW(L"dnsapi.dll");
  TrueDnsQueryRaw = dnsapi == nullptr ? nullptr :
      reinterpret_cast<decltype(TrueDnsQueryRaw)>(GetProcAddress(dnsapi, "DnsQueryRaw"));
  const int raw_required = TrueDnsQueryRaw != nullptr;
  const int raw = raw_required && EnvBoxAttach(&TrueDnsQueryRaw, HookDnsQueryRaw);
  ok += raw;
  g_dns_hooks_ready = network_params && adapters && free_a && free_w &&
      free_ex_a && free_ex_w && resolver_a && resolver_w && resolver_ex_a &&
      resolver_ex_w && resolver_ex_cancel && resolver_ex_overlapped && query_a &&
      query_w && query_utf8 && query_ex && g_dns_cancel_hook_attached &&
      (!raw_required || raw);
  return ok;
}
