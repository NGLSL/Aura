// Network Guard (ticket 56) - session/process-tree direct UDP constraint.
//
// Strict WebRTC policy denies non-loopback UDP from this injected process tree,
// except UDP/53 to the Profile DNS resolver allowlist (never "any :53").
//
// Process-scoped: only EnvBox-injected processes are hooked (no WFP driver,
// no Host firewall change). Loopback UDP stays open so local proxies work.
//
// Covers the Winsock send paths that actually leave the process:
//   sendto / WSASendTo          (connectionless, explicit dest)
//   connect + send / WSASend    (connected UDP)
//   WSASendMsg                  (WSA_MSG with optional name)
// TCP shares send/WSASend - guarded by per-socket SOCK_DGRAM state.
//
// Strict is NOT Fail Open: incomplete attach or an untrackable UDP socket is
// Startup Fail / refuse-the-socket (no Balanced downgrade). Policy token is
// read from ENVBOX_WEBRTC_POLICY (Host Environment Block / UpsertProfileKeys).
// RuntimeProfile::webrtc_policy is used when the field is present (UTF-16
// token: host / public_interface_only / proxy_only / strict).
//
// Include order: winsock2.h MUST precede windows.h (via hooks.h).

#include <winsock2.h>
#include <ws2tcpip.h>
#include <mswsock.h>
#include <iphlpapi.h>

#include "hooks.h"

#include <string.h>

#include "audit.h"
#include "runtime_profile.h"

#pragma comment(lib, "ws2_32.lib")

// ---------------------------------------------------------------------------
// True pointers
// ---------------------------------------------------------------------------

static SOCKET(WSAAPI* True_socket)(int, int, int) = socket;
static SOCKET(WSAAPI* True_WSASocketW)(int, int, int, LPWSAPROTOCOL_INFOW, GROUP,
                                       DWORD) = WSASocketW;
static int(WSAAPI* Trueconnect)(SOCKET, const struct sockaddr*, int) = connect;
static int(WSAAPI* TrueWSAConnect)(SOCKET, const struct sockaddr*, int,
                                   LPWSABUF, LPWSABUF, LPQOS, LPQOS) = WSAConnect;
static int(WSAAPI* Trueclosesocket)(SOCKET) = closesocket;
static int(WSAAPI* Truesend)(SOCKET, const char*, int, int) = send;
static int(WSAAPI* TrueWSASend)(SOCKET, LPWSABUF, DWORD, LPDWORD, DWORD,
                                LPWSAOVERLAPPED,
                                LPWSAOVERLAPPED_COMPLETION_ROUTINE) = WSASend;
static int(WSAAPI* Truesendto)(SOCKET, const char*, int, int,
                               const struct sockaddr*, int) = sendto;
static int(WSAAPI* TrueWSASendTo)(SOCKET, LPWSABUF, DWORD, LPDWORD, DWORD,
                                  const struct sockaddr*, int, LPWSAOVERLAPPED,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE) = WSASendTo;
static int(WSAAPI* TrueWSASendMsg)(SOCKET, LPWSAMSG, DWORD, LPDWORD,
                                   LPWSAOVERLAPPED,
                                   LPWSAOVERLAPPED_COMPLETION_ROUTINE) = WSASendMsg;

// ---------------------------------------------------------------------------
// Policy + DNS allowlist
// ---------------------------------------------------------------------------

// 0=host, 1=public_interface_only, 2=proxy_only, 3=strict
static int g_webrtc_policy = 0;

// UDP/53 destinations allowed under Strict (Profile resolvers + host DNS when
// the Profile does not pin a list). Empty allowlist => no external UDP/53.
#ifndef ENVBOX_UDP_DNS_ALLOW_MAX
#define ENVBOX_UDP_DNS_ALLOW_MAX 16
#endif
static sockaddr_storage g_dns_allow[ENVBOX_UDP_DNS_ALLOW_MAX];
static int g_dns_allow_len[ENVBOX_UDP_DNS_ALLOW_MAX];
static int g_dns_allow_count = 0;

// ---------------------------------------------------------------------------
// Per-socket UDP state (connected send path)
// ---------------------------------------------------------------------------

#ifndef ENVBOX_UDP_SOCK_MAX
#define ENVBOX_UDP_SOCK_MAX 256
#endif

struct UdpSock {
  SOCKET s;
  int in_use;
  int connected;
  int remote_len;
  sockaddr_storage remote;
};

static UdpSock g_udp_socks[ENVBOX_UDP_SOCK_MAX];
static CRITICAL_SECTION g_udp_cs;
static int g_udp_cs_ready = 0;

static void UdpCsInit() {
  if (!g_udp_cs_ready) {
    InitializeCriticalSection(&g_udp_cs);
    g_udp_cs_ready = 1;
  }
}

static UdpSock* UdpFind(SOCKET s, int create_if_missing) {
  if (s == INVALID_SOCKET) return nullptr;
  UdpCsInit();
  EnterCriticalSection(&g_udp_cs);
  UdpSock* free_slot = nullptr;
  UdpSock* found = nullptr;
  for (int i = 0; i < ENVBOX_UDP_SOCK_MAX; i++) {
    if (g_udp_socks[i].in_use && g_udp_socks[i].s == s) {
      found = &g_udp_socks[i];
      break;
    }
    if (!g_udp_socks[i].in_use && free_slot == nullptr) {
      free_slot = &g_udp_socks[i];
    }
  }
  if (found == nullptr && create_if_missing && free_slot != nullptr) {
    free_slot->s = s;
    free_slot->in_use = 1;
    free_slot->connected = 0;
    free_slot->remote_len = 0;
    found = free_slot;
  }
  LeaveCriticalSection(&g_udp_cs);
  return found;
}

static void UdpForget(SOCKET s) {
  UdpCsInit();
  EnterCriticalSection(&g_udp_cs);
  for (int i = 0; i < ENVBOX_UDP_SOCK_MAX; i++) {
    if (g_udp_socks[i].in_use && g_udp_socks[i].s == s) {
      g_udp_socks[i].in_use = 0;
      g_udp_socks[i].connected = 0;
      g_udp_socks[i].s = INVALID_SOCKET;
    }
  }
  LeaveCriticalSection(&g_udp_cs);
}

static void UdpMarkConnected(SOCKET s, const struct sockaddr* name, int namelen) {
  if (name == nullptr || namelen <= 0) return;
  UdpSock* slot = UdpFind(s, 1);
  if (slot == nullptr) return;
  UdpCsInit();
  EnterCriticalSection(&g_udp_cs);
  slot->connected = 1;
  slot->remote_len = 0;
  if (namelen <= (int)sizeof(sockaddr_storage)) {
    memcpy(&slot->remote, name, (size_t)namelen);
    slot->remote_len = namelen;
  }
  LeaveCriticalSection(&g_udp_cs);
}

// ---------------------------------------------------------------------------
// Address helpers
// ---------------------------------------------------------------------------

static int IsLoopbackAddr(const struct sockaddr* addr, int len) {
  if (addr == nullptr) return 0;
  if (addr->sa_family == AF_INET && len >= (int)sizeof(sockaddr_in)) {
    const sockaddr_in* v4 = (const sockaddr_in*)addr;
    unsigned int a = ntohl(v4->sin_addr.s_addr);
    return (a >> 24) == 127;  // 127.0.0.0/8
  }
  if (addr->sa_family == AF_INET6 && len >= (int)sizeof(sockaddr_in6)) {
    const sockaddr_in6* v6 = (const sockaddr_in6*)addr;
    static const unsigned char loop[16] = {0, 0, 0, 0, 0, 0, 0, 0,
                                           0, 0, 0, 0, 0, 0, 0, 1};
    if (memcmp(&v6->sin6_addr, loop, 16) == 0) return 1;  // ::1
    // IPv4-mapped ::ffff:127.0.0.0/8 also counts as loopback.
    static const unsigned char v4mapped[12] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                                               0xff, 0xff};
    if (memcmp(&v6->sin6_addr, v4mapped, 12) == 0) {
      const unsigned char* b = (const unsigned char*)&v6->sin6_addr;
      return b[12] == 127;
    }
  }
  return 0;
}

static int AddrPort(const struct sockaddr* addr, int len) {
  if (addr == nullptr) return -1;
  if (addr->sa_family == AF_INET && len >= (int)sizeof(sockaddr_in)) {
    return ntohs(((const sockaddr_in*)addr)->sin_port);
  }
  if (addr->sa_family == AF_INET6 && len >= (int)sizeof(sockaddr_in6)) {
    return ntohs(((const sockaddr_in6*)addr)->sin6_port);
  }
  return -1;
}

static int AddrEqualForDns(const struct sockaddr* a, int alen,
                           const struct sockaddr* b, int blen) {
  if (a == nullptr || b == nullptr) return 0;
  // Compare IP only (DNS port is checked separately).
  if (a->sa_family == AF_INET && b->sa_family == AF_INET) {
    if (alen < (int)sizeof(sockaddr_in) || blen < (int)sizeof(sockaddr_in)) {
      return 0;
    }
    return ((const sockaddr_in*)a)->sin_addr.s_addr ==
           ((const sockaddr_in*)b)->sin_addr.s_addr;
  }
  if (a->sa_family == AF_INET6 && b->sa_family == AF_INET6) {
    if (alen < (int)sizeof(sockaddr_in6) || blen < (int)sizeof(sockaddr_in6)) {
      return 0;
    }
    return memcmp(&((const sockaddr_in6*)a)->sin6_addr,
                  &((const sockaddr_in6*)b)->sin6_addr, 16) == 0;
  }
  // Mixed family: only IPv4-mapped IPv6 vs IPv4.
  const sockaddr_in6* v6 = nullptr;
  const sockaddr_in* v4 = nullptr;
  if (a->sa_family == AF_INET6 && b->sa_family == AF_INET) {
    v6 = (const sockaddr_in6*)a;
    v4 = (const sockaddr_in*)b;
    if (alen < (int)sizeof(sockaddr_in6)) return 0;
  } else if (a->sa_family == AF_INET && b->sa_family == AF_INET6) {
    v4 = (const sockaddr_in*)a;
    v6 = (const sockaddr_in6*)b;
    if (blen < (int)sizeof(sockaddr_in6)) return 0;
  } else {
    return 0;
  }
  static const unsigned char v4mapped[12] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                                             0xff, 0xff};
  const unsigned char* b6 = (const unsigned char*)&v6->sin6_addr;
  if (memcmp(b6, v4mapped, 12) != 0) return 0;
  unsigned int mapped = (unsigned int)b6[12] << 24 | (unsigned int)b6[13] << 16 |
                        (unsigned int)b6[14] << 8 | (unsigned int)b6[15];
  return htonl(mapped) == v4->sin_addr.s_addr;
}

// ---------------------------------------------------------------------------
// DNS resolver allowlist (UDP/53 is not a blanket hole)
// ---------------------------------------------------------------------------

static int ParseDnsServerA(const char* text, sockaddr_storage* out, int* out_len) {
  if (text == nullptr || text[0] == '\0' || out == nullptr || out_len == nullptr) {
    return 0;
  }
  memset(out, 0, sizeof(*out));
  sockaddr_in v4 = {};
  v4.sin_family = AF_INET;
  if (inet_pton(AF_INET, text, &v4.sin_addr) == 1) {
    memcpy(out, &v4, sizeof(v4));
    *out_len = (int)sizeof(v4);
    return 1;
  }
  sockaddr_in6 v6 = {};
  v6.sin6_family = AF_INET6;
  if (inet_pton(AF_INET6, text, &v6.sin6_addr) == 1) {
    memcpy(out, &v6, sizeof(v6));
    *out_len = (int)sizeof(v6);
    return 1;
  }
  return 0;
}

static void DnsAllowAdd(const struct sockaddr* addr, int len) {
  if (addr == nullptr || len <= 0 || len > (int)sizeof(sockaddr_storage)) return;
  if (g_dns_allow_count >= ENVBOX_UDP_DNS_ALLOW_MAX) return;
  for (int i = 0; i < g_dns_allow_count; i++) {
    if (AddrEqualForDns((const struct sockaddr*)&g_dns_allow[i], g_dns_allow_len[i],
                        addr, len)) {
      return;
    }
  }
  memcpy(&g_dns_allow[g_dns_allow_count], addr, (size_t)len);
  g_dns_allow_len[g_dns_allow_count] = len;
  g_dns_allow_count++;
}

static void DnsAllowAddText(const char* text) {
  sockaddr_storage ss = {};
  int slen = 0;
  if (ParseDnsServerA(text, &ss, &slen)) {
    DnsAllowAdd((const struct sockaddr*)&ss, slen);
  }
}

// Host resolver list via GetNetworkParams. Built during install BEFORE
// DetourTransactionCommit, so this still sees the unhooked prologue. Do not
// call after commit expecting a bypass - a local fn ptr does not escape Detours.
static void DnsAllowAddHostResolvers() {
  static DWORD(WINAPI* TrueGetNetworkParamsLocal)(PFIXED_INFO, PULONG) =
      GetNetworkParams;
  ULONG len = 0;
  if (TrueGetNetworkParamsLocal(nullptr, &len) != ERROR_BUFFER_OVERFLOW ||
      len == 0) {
    return;
  }
  PFIXED_INFO info = (PFIXED_INFO)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, len);
  if (info == nullptr) return;
  if (TrueGetNetworkParamsLocal(info, &len) == ERROR_SUCCESS) {
    PIP_ADDR_STRING cur = &info->DnsServerList;
    int guard = 0;
    while (cur != nullptr && guard++ < 32) {
      DnsAllowAddText(cur->IpAddress.String);
      cur = cur->Next;
    }
  }
  HeapFree(GetProcessHeap(), 0, info);
}

static void BuildDnsAllowlist(const RuntimeProfile* pfl) {
  g_dns_allow_count = 0;
  if (pfl != nullptr && pfl->dns_mode == 1) {
    // VirtualView: ONLY Profile dns_servers. Empty list closes external UDP/53
    // (DoH / local stub). Loopback stays open via IsLoopbackAddr regardless.
    for (int i = 0; i < pfl->dns_server_count && i < ENVBOX_DNS_MAX; i++) {
      DnsAllowAddText(pfl->dns_servers[i]);
    }
  } else {
    // Host mode (or no Profile): system DNS actually sends to host resolvers.
    // To close external UDP/53 use VirtualView with empty or loopback-only
    // dns_servers - never "any :53", never a silent host fallback.
    DnsAllowAddHostResolvers();
  }
}

static int IsAllowedDnsDest(const struct sockaddr* addr, int len) {
  for (int i = 0; i < g_dns_allow_count; i++) {
    if (AddrEqualForDns(addr, len, (const struct sockaddr*)&g_dns_allow[i],
                        g_dns_allow_len[i])) {
      return 1;
    }
  }
  return 0;
}

// ---------------------------------------------------------------------------
// Policy load + deny predicate
// ---------------------------------------------------------------------------

static int ParseWebrtcTokenA(const char* tok) {
  if (tok == nullptr || tok[0] == '\0') return 0;
  if (_stricmp(tok, "host") == 0) return 0;
  if (_stricmp(tok, "public_interface_only") == 0) return 1;
  if (_stricmp(tok, "proxy_only") == 0) return 2;
  if (_stricmp(tok, "strict") == 0) return 3;
  return 0;
}

static int ParseWebrtcTokenW(const wchar_t* tok) {
  if (tok == nullptr || tok[0] == L'\0') return 0;
  if (_wcsicmp(tok, L"host") == 0) return 0;
  if (_wcsicmp(tok, L"public_interface_only") == 0) return 1;
  if (_wcsicmp(tok, L"proxy_only") == 0) return 2;
  if (_wcsicmp(tok, L"strict") == 0) return 3;
  return 0;
}

static void LoadWebrtcPolicy() {
  // Environment fallback is always written for profile-mode roots/children.
  char env[32] = {};
  if (GetEnvironmentVariableA("ENVBOX_WEBRTC_POLICY", env, sizeof(env)) > 0) {
    g_webrtc_policy = ParseWebrtcTokenA(env);
    return;
  }
  // IPC / packaged path: RuntimeProfile carries the token.
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr) {
    g_webrtc_policy = ParseWebrtcTokenW(pfl->webrtc_policy);
  }
}

// Strict: deny non-loopback UDP except UDP/53 to the DNS resolver allowlist.
// Returns 1 when the send/connect must be blocked.
static int ShouldDenyUdp(const struct sockaddr* addr, int len) {
  if (g_webrtc_policy != 3) return 0;
  if (IsLoopbackAddr(addr, len)) return 0;
  // UDP/53 is allowed ONLY to Profile/host resolver IPs - never any :53.
  // STUN/TURN/any other port is denied (never a well-known-port blocklist;
  // the deny is the default and DNS is the explicit exception).
  if (AddrPort(addr, len) == 53) {
    return IsAllowedDnsDest(addr, len) ? 0 : 1;
  }
  return 1;
}

static void AuditUdpDeny(const char* api) {
  const RuntimeProfile* pfl = EnvBoxProfile();
  if (pfl != nullptr && pfl->audit) {
    // Stable name (envbox-core AUDIT_API_NETWORK_UDP_DENY).
    EnvBoxAuditEvent(api, 1, "NetworkGuardUdpDeny");
  }
}

static int RejectSend(const char* api) {
  AuditUdpDeny(api);
  WSASetLastError(WSAEACCES);
  return 1;
}

static int SockIsUdp(SOCKET s) {
  int type = 0;
  int len = (int)sizeof(type);
  if (getsockopt(s, SOL_SOCKET, SO_TYPE, (char*)&type, &len) != 0) {
    return 0;
  }
  return (type & 0xff) == SOCK_DGRAM;
}

static int AddrIsUdpTarget(SOCKET s, const struct sockaddr* name, int namelen) {
  // Only SOCK_DGRAM is constrained. A TCP connect/sendto with a dest name
  // must never be treated as a UDP peer (that blocked all HTTPS under Strict).
  (void)name;
  (void)namelen;
  return SockIsUdp(s);
}

// Connected / no-dest send path. Untracked sockets are re-typed via SO_TYPE
// so a missed table entry cannot become a UDP escape. Strict fails closed.
static int DenyConnectedSend(SOCKET s, const char* api) {
  if (g_webrtc_policy != 3) return 0;
  UdpSock* slot = UdpFind(s, 0);
  if (slot != nullptr) {
    int deny = 0;
    UdpCsInit();
    EnterCriticalSection(&g_udp_cs);
    if (slot->in_use && slot->connected && slot->remote_len > 0) {
      deny = ShouldDenyUdp((const struct sockaddr*)&slot->remote, slot->remote_len);
    } else if (slot->in_use) {
      deny = 1;
    }
    LeaveCriticalSection(&g_udp_cs);
    return deny ? RejectSend(api) : 0;
  }
  if (!SockIsUdp(s)) return 0;
  sockaddr_storage peer = {};
  int plen = (int)sizeof(peer);
  if (getpeername(s, (struct sockaddr*)&peer, &plen) == 0) {
    return ShouldDenyUdp((const struct sockaddr*)&peer, plen) ? RejectSend(api)
                                                             : 0;
  }
  return RejectSend(api);
}

// Explicit dest (sendto / WSASendTo / WSASendMsg name). to=NULL is valid on a
// connected UDP socket - fall back to the peer, never treat as "any dest".
// TCP sockets with an explicit dest are not UDP sends - leave them alone.
static int DenyDestSend(SOCKET s, const struct sockaddr* to, int tolen,
                        const char* api) {
  if (g_webrtc_policy != 3) return 0;
  if (!SockIsUdp(s)) return 0;
  if (to != nullptr && tolen > 0) {
    return ShouldDenyUdp(to, tolen) ? RejectSend(api) : 0;
  }
  return DenyConnectedSend(s, api);
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

static int TrackUdpSocket(SOCKET s) {
  if (UdpFind(s, 1) != nullptr) return 1;
  if (g_webrtc_policy == 3) {
    // Table full: cannot track - refuse the UDP socket (no silent bypass).
    Trueclosesocket(s);
    WSASetLastError(WSAENOBUFS);
    return 0;
  }
  return 1;
}

static SOCKET WSAAPI Hook_socket(int af, int type, int protocol) {
  SOCKET s = True_socket(af, type, protocol);
  if (s != INVALID_SOCKET && (type & 0xff) == SOCK_DGRAM) {
    if (!TrackUdpSocket(s)) return INVALID_SOCKET;
  }
  return s;
}

static SOCKET WSAAPI Hook_WSASocketW(int af, int type, int protocol,
                                     LPWSAPROTOCOL_INFOW info, GROUP g,
                                     DWORD flags) {
  SOCKET s = True_WSASocketW(af, type, protocol, info, g, flags);
  if (s != INVALID_SOCKET && (type & 0xff) == SOCK_DGRAM) {
    if (!TrackUdpSocket(s)) return INVALID_SOCKET;
  }
  return s;
}

static SOCKET(WSAAPI* True_WSASocketA)(int, int, int, LPWSAPROTOCOL_INFOA, GROUP,
                                       DWORD) = WSASocketA;

static SOCKET WSAAPI Hook_WSASocketA(int af, int type, int protocol,
                                     LPWSAPROTOCOL_INFOA info, GROUP g,
                                     DWORD flags) {
  SOCKET s = True_WSASocketA(af, type, protocol, info, g, flags);
  if (s != INVALID_SOCKET && (type & 0xff) == SOCK_DGRAM) {
    if (!TrackUdpSocket(s)) return INVALID_SOCKET;
  }
  return s;
}

static int WSAAPI Hook_closesocket(SOCKET s) {
  UdpForget(s);
  return Trueclosesocket(s);
}

static int DenyConnect(SOCKET s, const struct sockaddr* name, int namelen,
                       const char* api) {
  if (g_webrtc_policy != 3) return 0;
  if (!AddrIsUdpTarget(s, name, namelen)) return 0;
  return ShouldDenyUdp(name, namelen) ? RejectSend(api) : 0;
}

static int WSAAPI Hook_connect(SOCKET s, const struct sockaddr* name, int namelen) {
  if (DenyConnect(s, name, namelen, "connect")) {
    return SOCKET_ERROR;
  }
  int r = Trueconnect(s, name, namelen);
  if (r == 0 && SockIsUdp(s)) {
    UdpMarkConnected(s, name, namelen);
  }
  return r;
}

static int WSAAPI Hook_WSAConnect(SOCKET s, const struct sockaddr* name,
                                  int namelen, LPWSABUF caller, LPWSABUF callee,
                                  LPQOS sqos, LPQOS gqos) {
  if (DenyConnect(s, name, namelen, "WSAConnect")) {
    return SOCKET_ERROR;
  }
  int r = TrueWSAConnect(s, name, namelen, caller, callee, sqos, gqos);
  if (r == 0 && SockIsUdp(s)) {
    UdpMarkConnected(s, name, namelen);
  }
  return r;
}

static int WSAAPI Hook_send(SOCKET s, const char* buf, int len, int flags) {
  if (DenyConnectedSend(s, "send")) {
    return SOCKET_ERROR;
  }
  return Truesend(s, buf, len, flags);
}

static int WSAAPI Hook_WSASend(SOCKET s, LPWSABUF bufs, DWORD count, LPDWORD sent,
                               DWORD flags, LPWSAOVERLAPPED ov,
                               LPWSAOVERLAPPED_COMPLETION_ROUTINE cr) {
  if (DenyConnectedSend(s, "WSASend")) {
    return SOCKET_ERROR;
  }
  return TrueWSASend(s, bufs, count, sent, flags, ov, cr);
}

static int WSAAPI Hook_sendto(SOCKET s, const char* buf, int len, int flags,
                              const struct sockaddr* to, int tolen) {
  if (DenyDestSend(s, to, tolen, "sendto")) {
    return SOCKET_ERROR;
  }
  return Truesendto(s, buf, len, flags, to, tolen);
}

static int WSAAPI Hook_WSASendTo(SOCKET s, LPWSABUF bufs, DWORD count,
                                 LPDWORD sent, DWORD flags,
                                 const struct sockaddr* to, int tolen,
                                 LPWSAOVERLAPPED ov,
                                 LPWSAOVERLAPPED_COMPLETION_ROUTINE cr) {
  if (DenyDestSend(s, to, tolen, "WSASendTo")) {
    return SOCKET_ERROR;
  }
  return TrueWSASendTo(s, bufs, count, sent, flags, to, tolen, ov, cr);
}

static int WSAAPI Hook_WSASendMsg(SOCKET s, LPWSAMSG msg, DWORD flags,
                                  LPDWORD sent, LPWSAOVERLAPPED ov,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE cr) {
  if (msg != nullptr) {
    if (DenyDestSend(s, msg->name, msg->namelen, "WSASendMsg")) {
      return SOCKET_ERROR;
    }
  }
  return TrueWSASendMsg(s, msg, flags, sent, ov, cr);
}

// Returns 1 when the process may start (non-Strict, or Strict guard armed).
// Returns 0 when Strict Network Guard cannot be armed - Startup Fail Policy
// forbids silently running without the UDP deny (no Balanced downgrade).
int EnvBoxInstallNetworkHooks() {
  LoadWebrtcPolicy();
  if (g_webrtc_policy != 3) {
    // Balanced / Host: Browser Policy only, no network-layer enforcement.
    return 1;
  }
  const RuntimeProfile* pfl = EnvBoxProfile();
  BuildDnsAllowlist(pfl);
  UdpCsInit();
  int ok = 0;
  // Critical for Strict: socket track + every send/connect path. Missing any
  // of these leaves a UDP escape; fail closed (Startup Fail) rather than run.
  ok += EnvBoxAttach(&True_socket, Hook_socket);
  ok += EnvBoxAttach(&True_WSASocketW, Hook_WSASocketW);
  ok += EnvBoxAttach(&True_WSASocketA, Hook_WSASocketA);
  ok += EnvBoxAttach(&Trueconnect, Hook_connect);
  ok += EnvBoxAttach(&TrueWSAConnect, Hook_WSAConnect);
  ok += EnvBoxAttach(&Trueclosesocket, Hook_closesocket);
  ok += EnvBoxAttach(&Truesend, Hook_send);
  ok += EnvBoxAttach(&TrueWSASend, Hook_WSASend);
  ok += EnvBoxAttach(&Truesendto, Hook_sendto);
  ok += EnvBoxAttach(&TrueWSASendTo, Hook_WSASendTo);
  ok += EnvBoxAttach(&TrueWSASendMsg, Hook_WSASendMsg);
  const int critical = 11;
  if (ok < critical) {
    OutputDebugStringA("EnvBox: Strict Network Guard attach incomplete (Startup Fail)\n");
    return 0;
  }
  return 1;
}
