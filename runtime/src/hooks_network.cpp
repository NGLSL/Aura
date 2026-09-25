// Network Guard (ticket 56) - session/process-tree direct UDP constraint.
//
// Strict WebRTC policy denies non-loopback UDP from this injected process tree.
// Process-scoped: only EnvBox-injected processes are hooked (no WFP driver,
// no Host firewall change). Loopback UDP stays open so local proxies work.
//
// Fail Open on hook errors. Policy token is read from ENVBOX_WEBRTC_POLICY
// (Host Environment Block / UpsertProfileKeys). RuntimeProfile::webrtc_policy
// is used when the field is present (UTF-16 token: host / public_interface_only
// / proxy_only / strict).
//
// Include order: winsock2.h MUST precede windows.h (via hooks.h).

#include <winsock2.h>
#include <ws2tcpip.h>

#include "hooks.h"

#include <string.h>

#include "audit.h"
#include "runtime_profile.h"

#pragma comment(lib, "ws2_32.lib")

static int(WINAPI* Truesendto)(SOCKET, const char*, int, int, const struct sockaddr*,
                               int) = sendto;
static int(WSAAPI* TrueWSASendTo)(SOCKET, LPWSABUF, DWORD, LPDWORD, DWORD,
                                  const struct sockaddr*, int, LPWSAOVERLAPPED,
                                  LPWSAOVERLAPPED_COMPLETION_ROUTINE) = WSASendTo;

// 0=host, 1=public_interface_only, 2=proxy_only, 3=strict
static int g_webrtc_policy = 0;

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
    return memcmp(&v6->sin6_addr, loop, 16) == 0;  // ::1
  }
  return 0;
}

// Strict: deny non-loopback UDP, except DNS (VirtualView / host resolver)
// so DNS is not a Network Guard target (spec: DNS stays out of scope).
// Returns 1 when the send must be blocked.
static int ShouldDenyUdp(const struct sockaddr* addr, int len) {
  if (g_webrtc_policy != 3) return 0;
  if (IsLoopbackAddr(addr, len)) return 0;
  // Allow DNS UDP (port 53): hooks_dns VirtualView uses sendto to resolvers.
  if (addr != nullptr) {
    if (addr->sa_family == AF_INET && len >= (int)sizeof(sockaddr_in)) {
      const sockaddr_in* v4 = (const sockaddr_in*)addr;
      if (ntohs(v4->sin_port) == 53) return 0;
    } else if (addr->sa_family == AF_INET6 && len >= (int)sizeof(sockaddr_in6)) {
      const sockaddr_in6* v6 = (const sockaddr_in6*)addr;
      if (ntohs(v6->sin6_port) == 53) return 0;
    }
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

static int WINAPI Hook_sendto(SOCKET s, const char* buf, int len, int flags,
                              const struct sockaddr* to, int tolen) {
  if (ShouldDenyUdp(to, tolen)) {
    AuditUdpDeny("sendto");
    WSASetLastError(WSAEACCES);
    return SOCKET_ERROR;
  }
  return Truesendto(s, buf, len, flags, to, tolen);
}

static int WSAAPI Hook_WSASendTo(SOCKET s, LPWSABUF bufs, DWORD count,
                                 LPDWORD sent, DWORD flags,
                                 const struct sockaddr* to, int tolen,
                                 LPWSAOVERLAPPED ov,
                                 LPWSAOVERLAPPED_COMPLETION_ROUTINE cr) {
  if (ShouldDenyUdp(to, tolen)) {
    AuditUdpDeny("WSASendTo");
    WSASetLastError(WSAEACCES);
    return SOCKET_ERROR;
  }
  return TrueWSASendTo(s, bufs, count, sent, flags, to, tolen, ov, cr);
}

int EnvBoxInstallNetworkHooks() {
  LoadWebrtcPolicy();
  if (g_webrtc_policy != 3) {
    // Balanced / Host: Browser Policy only, no network-layer enforcement.
    return 1;
  }
  int ok = 0;
  ok += EnvBoxAttach(&Truesendto, Hook_sendto);
  ok += EnvBoxAttach(&TrueWSASendTo, Hook_WSASendTo);
  return ok;
}
