// DNS View hooks (ticket 08) + DNS routing (ticket 25).
// DNS View: virtualize *read* of DNS config only (GetNetworkParams /
// GetAdaptersAddresses). DNS routing: under VirtualView, resolve names through
// Profile dns_servers via a minimal UDP/53 client. Host mode never hooks the
// resolve path. Fail Open to the original API when every Profile server is
// unreachable/timeout/truncated, or when NOERROR yields no A/AAAA after CNAME
// follow (max 8 hops). Authoritative NXDOMAIN (rcode=3) is the only definitive
// "name does not exist". Non-goals: WFP / LSP / DoH / port-53 redirect /
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
static void(WSAAPI* TrueFreeAddrInfoExA)(PADDRINFOEXA) = FreeAddrInfoExA;
static void(WSAAPI* TrueFreeAddrInfoExW)(PADDRINFOEXW) = FreeAddrInfoExW;
static DNS_STATUS(WINAPI* TrueDnsQuery_A)(PCSTR, WORD, DWORD, PVOID,
                                          PDNS_RECORD*, PVOID*) = DnsQuery_A;
static DNS_STATUS(WINAPI* TrueDnsQuery_W)(PCWSTR, WORD, DWORD, PVOID,
                                          PDNS_RECORD*, PVOID*) = DnsQuery_W;
static DNS_STATUS(WINAPI* TrueDnsQuery_UTF8)(PCSTR, WORD, DWORD, PVOID,
                                             PDNS_RECORD*,
                                             PVOID*) = DnsQuery_UTF8;
static void(WINAPI* TrueDnsFree)(PVOID, DNS_FREE_TYPE) = DnsFree;

// Process-immutable virtual DNS views (built once at hook install).
static int g_view_active = 0;
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

// Owned-allocation registry so freeaddrinfo / DnsRecordListFree can release
// our nodes even when the CRT heap differs from ws2_32/dnsapi.
#ifndef ENVBOX_OWNED_MAX
#define ENVBOX_OWNED_MAX 128
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
  if (pfl == nullptr || pfl->dns_mode != 1 || pfl->dns_server_count <= 0) {
    return;
  }
  int n = pfl->dns_server_count;
  if (n > ENVBOX_DNS_MAX) {
    n = ENVBOX_DNS_MAX;
  }
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
  if (name == nullptr || name[0] == '\0') {
    return 0;
  }
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

// One UDP query to a single server. timeout_ms is per-attempt (bounded).
// Returns 1 when a DNS response was parsed into *out (any rcode), else 0.
static int DnsQueryOne(const char* server_text, const char* qname,
                       unsigned qtype, DWORD timeout_ms, DnsAddrs* out) {
  unsigned char qbuf[512];
  int namelen = EncodeDnsName(qname, qbuf + 12, (int)sizeof(qbuf) - 12);
  if (namelen <= 0) {
    return 0;
  }

  static volatile LONG s_qid = 0;
  unsigned id = (unsigned)(GetCurrentProcessId() + InterlockedIncrement(&s_qid));
  memset(qbuf, 0, 12);
  WriteU16(qbuf + 0, id & 0xFFFF);
  WriteU16(qbuf + 2, 0x0100);  // RD
  WriteU16(qbuf + 4, 1);       // QDCOUNT
  WriteU16(qbuf + 6, 0);
  WriteU16(qbuf + 8, 0);
  WriteU16(qbuf + 10, 0);
  int qoff = 12 + namelen;
  WriteU16(qbuf + qoff, qtype);
  WriteU16(qbuf + qoff + 2, 1);  // IN
  qoff += 4;

  in_addr a4;
  in6_addr a6;
  int is_v4 = InetPtonA(AF_INET, server_text, &a4) == 1;
  int is_v6 = InetPtonA(AF_INET6, server_text, &a6) == 1;
  if (!is_v4 && !is_v6) {
    return 0;
  }

  SOCKET s = socket(is_v4 ? AF_INET : AF_INET6, SOCK_DGRAM, IPPROTO_UDP);
  if (s == INVALID_SOCKET) {
    return 0;
  }

  DWORD tv = timeout_ms;
  setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (const char*)&tv, sizeof(tv));
  setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, (const char*)&tv, sizeof(tv));

  int sent = 0;
  unsigned short dport = htons((unsigned short)DnsUdpPort());
  if (is_v4) {
    sockaddr_in dst;
    memset(&dst, 0, sizeof(dst));
    dst.sin_family = AF_INET;
    dst.sin_port = dport;
    dst.sin_addr = a4;
    sent = sendto(s, (const char*)qbuf, qoff, 0, (const sockaddr*)&dst,
                  sizeof(dst));
  } else {
    sockaddr_in6 dst6;
    memset(&dst6, 0, sizeof(dst6));
    dst6.sin6_family = AF_INET6;
    dst6.sin6_port = dport;
    dst6.sin6_addr = a6;
    sent = sendto(s, (const char*)qbuf, qoff, 0, (const sockaddr*)&dst6,
                  sizeof(dst6));
  }
  if (sent == SOCKET_ERROR) {
    closesocket(s);
    return 0;
  }

  unsigned char rbuf[1500];
  int rlen = recvfrom(s, (char*)rbuf, (int)sizeof(rbuf), 0, nullptr, nullptr);
  closesocket(s);
  if (rlen < 12) {
    return 0;
  }
  if (ReadU16(rbuf) != (id & 0xFFFF)) {
    return 0;
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
  unsigned qd = ReadU16(rbuf + 4);
  unsigned an = ReadU16(rbuf + 6);
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

  for (unsigned i = 0; i < an && off + 10 <= rlen; i++) {
    if (!SkipDnsName(rbuf, rlen, &off)) {
      break;
    }
    if (off + 10 > rlen) {
      break;
    }
    unsigned typ = ReadU16(rbuf + off);
    unsigned cls = ReadU16(rbuf + off + 2);
    unsigned rdlen = ReadU16(rbuf + off + 8);
    off += 10;
    if (cls != 1 || off + (int)rdlen > rlen) {
      off += (int)rdlen;
      continue;
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
  return 1;
}

// Route one name through Profile servers in order. Follows CNAME (max
// kDnsMaxCnameHops). Returns:
//   1 = definitive answer (addresses and/or authoritative NXDOMAIN)
//   0 = no definitive answer (Fail Open) -- truncated, unreachable, or
//       NOERROR without A/AAAA even after CNAME follow
static int DnsRouteName(const char* qname, int want_a, int want_aaaa,
                        DnsAddrs* out) {
  DnsAddrsClear(out);
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl == nullptr || pfl->dns_mode != 1 || pfl->dns_server_count <= 0) {
    return 0;
  }
  if (qname == nullptr || qname[0] == '\0') {
    return 0;
  }
  if (DnsCacheLookup(qname, want_a, want_aaaa, out)) {
    return 1;
  }

  char current[256];
  size_t qn = strlen(qname);
  if (qn >= sizeof(current)) {
    return 0;
  }
  memcpy(current, qname, qn + 1);

  ULONGLONG deadline = GetTickCount64() + kDnsTotalBudgetMs;
  int n = pfl->dns_server_count;
  if (n > ENVBOX_DNS_MAX) {
    n = ENVBOX_DNS_MAX;
  }

  for (int hop = 0; hop < kDnsMaxCnameHops; hop++) {
    int saw_nx = 0;
    int saw_ok = 0;
    int has_cname = 0;
    char next_name[256];
    next_name[0] = '\0';

    for (int si = 0; si < n; si++) {
      const char* server = pfl->dns_servers[si];

      if (want_a) {
        ULONGLONG now = GetTickCount64();
        if (now >= deadline) {
          break;
        }
        DWORD t = kDnsQueryTimeoutMs;
        ULONGLONG left = deadline - now;
        if (left < (ULONGLONG)t) {
          t = (DWORD)left;
        }
        if (t < 200) {
          t = 200;
        }
        DnsAddrs step;
        DnsAddrsClear(&step);
        if (DnsQueryOne(server, current, 1, t, &step)) {
          if (step.nxdomain) {
            saw_nx = 1;
          } else if (step.noerror) {
            saw_ok = 1;
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

      if (want_aaaa && !saw_nx) {
        ULONGLONG now = GetTickCount64();
        if (now >= deadline) {
          break;
        }
        DWORD t = kDnsQueryTimeoutMs;
        ULONGLONG left = deadline - now;
        if (left < (ULONGLONG)t) {
          t = (DWORD)left;
        }
        if (t < 200) {
          t = 200;
        }
        DnsAddrs step6;
        DnsAddrsClear(&step6);
        if (DnsQueryOne(server, current, 28, t, &step6)) {
          if (step6.nxdomain) {
            saw_nx = 1;
          } else if (step6.noerror) {
            saw_ok = 1;
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
      if (saw_nx || saw_ok) {
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
    // Not a name error -- Fail Open to the original API.
    return 0;
  }
  // CNAME hops exhausted without addresses: Fail Open.
  return 0;
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
                          int ai_flags, DnsAddrs* addrs, unsigned short* port) {
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

  if (!DnsRouteName(qname_utf8, want_a, want_aaaa, addrs)) {
    return EAI_AGAIN;  // caller Fail Opens
  }
  if (addrs->nxdomain && addrs->n_v4 == 0 && addrs->n_v6 == 0) {
    return EAI_NONAME;
  }
  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(addrs, ai_family, pairs, kDnsMaxAnswers * 2);
  if (np == 0) {
    if (addrs->n_v4 > 0 || addrs->n_v6 > 0) {
      // Addresses exist but none match ai_family.
      return EAI_NONAME;
    }
    // NOERROR without A/AAAA (CNAME-only / other type): Fail Open, never
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

static void FreeDnsRecordChain(PDNS_RECORD rec) {
  while (rec != nullptr) {
    PDNS_RECORD next = rec->pNext;
    if (rec->pName != nullptr) {
      OwnedFree(rec->pName);
    }
    OwnedFree(rec);
    rec = next;
  }
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

  // Numeric node / AI_NUMERICHOST / service-only: pass through unchanged.
  if (node == nullptr || node[0] == '\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeA(node) || !IsAsciiNameA(node)) {
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
    INT r = Truegetaddrinfo(node, service, hints, result);
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "fail-open node=%.64s", node);
    EnvBoxAuditEvent(api, 0, note);
    SetLastError(err);
    return r;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "nxdomain node=%.64s", node);
    EnvBoxAuditEvent(api, 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    INT r = Truegetaddrinfo(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "fail-open");
    SetLastError(err);
    return r;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOA chain = BuildChain<ADDRINFOA, char, decltype(&AllocAddrInfoA)>(
      pairs, np, port, family, stype, proto, flags, node, &AllocAddrInfoA);
  if (chain == nullptr) {
    INT r = Truegetaddrinfo(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "fail-open");
    SetLastError(err);
    return r;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoAChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route node=%.64s n=%d", node,
              np);
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

  if (node == nullptr || node[0] == L'\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeW(node)) {
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "numeric-or-passthrough");
    SetLastError(err);
    return r;
  }

  char node_u8[256];
  if (!WideToUtf8(node, node_u8, (int)sizeof(node_u8)) ||
      !IsAsciiNameA(node_u8)) {
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "fail-open");
    SetLastError(err);
    return r;
  }
  char svc_u8[64];
  svc_u8[0] = '\0';
  if (service != nullptr && service[0] != L'\0') {
    if (!WideToUtf8(service, svc_u8, (int)sizeof(svc_u8))) {
      INT r = TrueGetAddrInfoW(node, service, hints, result);
      EnvBoxAuditEvent(api, 0, "fail-open");
      SetLastError(err);
      return r;
    }
  }

  DnsAddrs addrs;
  unsigned short port = 0;
  int rc = ResolveRoutedA(node_u8, svc_u8, family, stype, proto, flags, &addrs,
                          &port);
  if (rc == EAI_AGAIN) {
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "fail-open node=%.64s", node_u8);
    EnvBoxAuditEvent(api, 0, note);
    SetLastError(err);
    return r;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "nxdomain node=%.64s", node_u8);
    EnvBoxAuditEvent(api, 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "fail-open");
    SetLastError(err);
    return r;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOW chain = BuildChain<ADDRINFOW, wchar_t, decltype(&AllocAddrInfoW)>(
      pairs, np, port, family, stype, proto, flags, node, &AllocAddrInfoW);
  if (chain == nullptr) {
    INT r = TrueGetAddrInfoW(node, service, hints, result);
    EnvBoxAuditEvent(api, 0, "fail-open");
    SetLastError(err);
    return r;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoWChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route node=%.64s n=%d",
              node_u8, np);
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
  // Async completion is out of scope: Fail Open to the original API.
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

  if (name == nullptr || name[0] == '\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeA(name) || !IsAsciiNameA(name)) {
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
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "fail-open node=%.64s", name);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, note);
    SetLastError(err);
    return r;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "nxdomain node=%.64s", name);
    EnvBoxAuditEvent("GetAddrInfoExA", 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, "fail-open");
    SetLastError(err);
    return r;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOEXA chain =
      BuildChain<ADDRINFOEXA, char, decltype(&AllocAddrInfoExA)>(
          pairs, np, port, family, stype, proto, flags, name,
          &AllocAddrInfoExA);
  if (chain == nullptr) {
    INT r = TrueGetAddrInfoExA(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExA", 0, "fail-open");
    SetLastError(err);
    return r;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoExAChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route node=%.64s n=%d", name,
              np);
  EnvBoxAuditEvent("GetAddrInfoExA", 1, note);
  SetLastError(err);
  return 0;
}

static INT WSAAPI HookGetAddrInfoExW(
    PCWSTR name, PCWSTR service, DWORD dw_name_space, LPGUID nlp_id,
    const ADDRINFOEXW* hints, PADDRINFOEXW* result, struct timeval* timeout,
    LPOVERLAPPED overlapped,
    LPLOOKUPSERVICE_COMPLETION_ROUTINE completion, LPHANDLE name_handle) {
  DWORD err = GetLastError();
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

  if (name == nullptr || name[0] == L'\0' || (flags & AI_NUMERICHOST) ||
      IsNumericNodeW(name)) {
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
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "fail-open");
    SetLastError(err);
    return r;
  }
  char svc_u8[64];
  svc_u8[0] = '\0';
  if (service != nullptr && service[0] != L'\0') {
    if (!WideToUtf8(service, svc_u8, (int)sizeof(svc_u8))) {
      INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                                 result, timeout, overlapped, completion,
                                 name_handle);
      EnvBoxAuditEvent("GetAddrInfoExW", 0, "fail-open");
      SetLastError(err);
      return r;
    }
  }

  DnsAddrs addrs;
  unsigned short port = 0;
  int rc = ResolveRoutedA(name_u8, svc_u8, family, stype, proto, flags, &addrs,
                          &port);
  if (rc == EAI_AGAIN) {
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "fail-open node=%.64s", name_u8);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, note);
    SetLastError(err);
    return r;
  }
  if (rc == EAI_NONAME) {
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "nxdomain node=%.64s", name_u8);
    EnvBoxAuditEvent("GetAddrInfoExW", 1, note);
    SetLastError(err);
    return EAI_NONAME;
  }
  if (rc != 0) {
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "fail-open");
    SetLastError(err);
    return r;
  }

  AddrPair pairs[kDnsMaxAnswers * 2];
  int np = CollectPairs(&addrs, family, pairs, kDnsMaxAnswers * 2);
  PADDRINFOEXW chain =
      BuildChain<ADDRINFOEXW, wchar_t, decltype(&AllocAddrInfoExW)>(
          pairs, np, port, family, stype, proto, flags, name,
          &AllocAddrInfoExW);
  if (chain == nullptr) {
    INT r = TrueGetAddrInfoExW(name, service, dw_name_space, nlp_id, hints,
                               result, timeout, overlapped, completion,
                               name_handle);
    EnvBoxAuditEvent("GetAddrInfoExW", 0, "fail-open");
    SetLastError(err);
    return r;
  }
  if (result != nullptr) {
    *result = chain;
  } else {
    FreeAddrInfoExWChain(chain);
  }
  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route node=%.64s n=%d",
              name_u8, np);
  EnvBoxAuditEvent("GetAddrInfoExW", 1, note);
  SetLastError(err);
  return 0;
}

// ---------------------------------------------------------------------------
// DnsQuery_A / W / UTF8 (A + AAAA). DnsQueryEx is async-shaped: Fail Open and
// not hooked (audit documents the gap at install).
// ---------------------------------------------------------------------------

enum DnsQueryFlavor {
  kDnsFlavorA = 0,
  kDnsFlavorW = 1,
  kDnsFlavorUtf8 = 2,
};

// Always Fail Open / pass through to the matching original API.
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

static PDNS_RECORD AllocDnsRecord(const char* name_a, const wchar_t* name_w,
                                  int is_wide, WORD wtype, const void* rdata,
                                  WORD rdlen) {
  PDNS_RECORD rec = (PDNS_RECORD)OwnedAlloc(sizeof(DNS_RECORD));
  if (rec == nullptr) {
    return nullptr;
  }
  if (is_wide && name_w != nullptr) {
    size_t n = wcslen(name_w) + 1;
    rec->pName = (PSTR)OwnedAlloc(n * sizeof(wchar_t));
    if (rec->pName != nullptr) {
      memcpy(rec->pName, name_w, n * sizeof(wchar_t));
    }
  } else if (name_a != nullptr) {
    size_t n = strlen(name_a) + 1;
    rec->pName = (PSTR)OwnedAlloc(n);
    if (rec->pName != nullptr) {
      memcpy(rec->pName, name_a, n);
    }
  }
  rec->wType = wtype;
  rec->wDataLength = rdlen;
  rec->Flags.DW = 0;
  rec->Flags.S.Section = DnsSectionAnswer;
  rec->Flags.S.CharSet = is_wide ? DnsCharSetUnicode : DnsCharSetUtf8;
  rec->dwTtl = 60;
  rec->dwReserved = 0;
  rec->pNext = nullptr;
  if (wtype == DNS_TYPE_A && rdlen == sizeof(DNS_A_DATA)) {
    memcpy(&rec->Data.A, rdata, sizeof(DNS_A_DATA));
  } else if (wtype == DNS_TYPE_AAAA && rdlen == sizeof(DNS_AAAA_DATA)) {
    memcpy(&rec->Data.AAAA, rdata, sizeof(DNS_AAAA_DATA));
  }
  return rec;
}

static DNS_STATUS RouteDnsQuery(const char* name_u8, WORD wtype, DWORD opts,
                                PDNS_RECORD* out, PVOID* reserved,
                                DnsQueryFlavor flavor, const wchar_t* name_w,
                                const char* api) {
  DWORD err = GetLastError();
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
  // Numeric / empty / non-ASCII: pass through (same policy as getaddrinfo).
  if (name_u8 == nullptr || name_u8[0] == '\0' || IsNumericNodeA(name_u8) ||
      !IsAsciiNameA(name_u8)) {
    DNS_STATUS st =
        CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    EnvBoxAuditEvent(api, 0, "numeric-or-passthrough");
    SetLastError(err);
    return st;
  }
  if (wtype != DNS_TYPE_A && wtype != DNS_TYPE_AAAA) {
    DNS_STATUS st =
        CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    EnvBoxAuditEvent(api, 0, "fail-open-type");
    SetLastError(err);
    return st;
  }

  DnsAddrs addrs;
  int want_a = (wtype == DNS_TYPE_A);
  int want_aaaa = (wtype == DNS_TYPE_AAAA);
  if (!DnsRouteName(name_u8, want_a, want_aaaa, &addrs)) {
    DNS_STATUS st =
        CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "fail-open node=%.64s", name_u8);
    EnvBoxAuditEvent(api, 0, note);
    SetLastError(err);
    return st;
  }
  if (addrs.nxdomain && addrs.n_v4 == 0 && addrs.n_v6 == 0) {
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "nxdomain node=%.64s", name_u8);
    EnvBoxAuditEvent(api, 1, note);
    SetLastError(err);
    return DNS_ERROR_RCODE_NAME_ERROR;
  }
  if (addrs.n_v4 == 0 && addrs.n_v6 == 0) {
    // NOERROR without A/AAAA must not become NAME_ERROR.
    DNS_STATUS st =
        CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    char note[128];
    _snprintf_s(note, sizeof(note), _TRUNCATE, "fail-open node=%.64s", name_u8);
    EnvBoxAuditEvent(api, 0, note);
    SetLastError(err);
    return st;
  }

  PDNS_RECORD head = nullptr;
  PDNS_RECORD tail = nullptr;
  int n = 0;
  int alloc_failed = 0;
  for (int i = 0; i < addrs.n_v4; i++) {
    DNS_A_DATA a;
    memset(&a, 0, sizeof(a));
    memcpy(&a.IpAddress, &addrs.v4[i], 4);
    PDNS_RECORD rec = AllocDnsRecord(name_u8, name_w, flavor == kDnsFlavorW,
                                     DNS_TYPE_A, &a, (WORD)sizeof(DNS_A_DATA));
    if (rec == nullptr) {
      alloc_failed = 1;
      break;
    }
    if (tail == nullptr) {
      head = rec;
    } else {
      tail->pNext = rec;
    }
    tail = rec;
    n++;
  }
  for (int i = 0; !alloc_failed && i < addrs.n_v6; i++) {
    DNS_AAAA_DATA a;
    memset(&a, 0, sizeof(a));
    memcpy(&a.Ip6Address, &addrs.v6[i], 16);
    PDNS_RECORD rec =
        AllocDnsRecord(name_u8, name_w, flavor == kDnsFlavorW, DNS_TYPE_AAAA,
                       &a, (WORD)sizeof(DNS_AAAA_DATA));
    if (rec == nullptr) {
      alloc_failed = 1;
      break;
    }
    if (tail == nullptr) {
      head = rec;
    } else {
      tail->pNext = rec;
    }
    tail = rec;
    n++;
  }

  if (alloc_failed || head == nullptr) {
    FreeDnsRecordChain(head);
    DNS_STATUS st =
        CallTrueDnsQuery(flavor, name_u8, name_w, wtype, opts, out, reserved);
    EnvBoxAuditEvent(api, 0, "fail-open");
    SetLastError(err);
    return st;
  }

  if (out != nullptr) {
    *out = head;
  } else {
    FreeDnsRecordChain(head);
  }

  char note[128];
  _snprintf_s(note, sizeof(note), _TRUNCATE, "dns-route node=%.64s n=%d",
              name_u8, n);
  EnvBoxAuditEvent(api, 1, note);
  SetLastError(err);
  return 0;
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
    DNS_STATUS st = TrueDnsQuery_W(name, wtype, opts, nullptr, out, reserved);
    EnvBoxAuditEvent("DnsQuery_W", 0, "fail-open");
    SetLastError(err);
    return st;
  }
  DNS_STATUS st = RouteDnsQuery(name_u8, wtype, opts, out, reserved,
                                kDnsFlavorW, name, "DnsQuery_W");
  SetLastError(err);
  return st;
}

// DnsRecordListFree is a macro over DnsFree(..., DnsFreeRecordList).
static void WINAPI HookDnsFree(PVOID rec, DNS_FREE_TYPE type) {
  DWORD err = GetLastError();
  if (!OwnedHas(rec)) {
    TrueDnsFree(rec, type);
    SetLastError(err);
    return;
  }
  if (type == DnsFreeRecordList) {
    FreeDnsRecordChain((PDNS_RECORD)rec);
  } else {
    OwnedFree(rec);
  }
  SetLastError(err);
}

int EnvBoxInstallDnsHooks() {
  BuildDnsView();
  int ok = 0;
  ok += EnvBoxAttach(&TrueGetNetworkParams, HookGetNetworkParams);
  ok += EnvBoxAttach(&TrueGetAdaptersAddresses, HookGetAdaptersAddresses);

  // DNS routing only under VirtualView + Profile servers (Host path untouched).
  if (!g_view_active) {
    EnvBoxAuditEvent("DnsQueryEx", 0, "dns-host");
    return ok;
  }

  ok += EnvBoxAttach(&Truegetaddrinfo, Hookgetaddrinfo);
  ok += EnvBoxAttach(&TrueGetAddrInfoW, HookGetAddrInfoW);
  ok += EnvBoxAttach(&Truefreeaddrinfo, Hookfreeaddrinfo);
  ok += EnvBoxAttach(&TrueFreeAddrInfoW, HookFreeAddrInfoW);
  ok += EnvBoxAttach(&TrueGetAddrInfoExA, HookGetAddrInfoExA);
  ok += EnvBoxAttach(&TrueGetAddrInfoExW, HookGetAddrInfoExW);
  ok += EnvBoxAttach(&TrueFreeAddrInfoExA, HookFreeAddrInfoExA);
  ok += EnvBoxAttach(&TrueFreeAddrInfoExW, HookFreeAddrInfoExW);
  ok += EnvBoxAttach(&TrueDnsQuery_A, HookDnsQuery_A);
  ok += EnvBoxAttach(&TrueDnsQuery_W, HookDnsQuery_W);
  ok += EnvBoxAttach(&TrueDnsQuery_UTF8, HookDnsQuery_UTF8);
  ok += EnvBoxAttach(&TrueDnsFree, HookDnsFree);
  // DnsQueryEx is async/completion-shaped: not hooked (Fail Open by design).
  EnvBoxAuditEvent("DnsQueryEx", 0, "fail-open-unhooked");
  return ok;
}
