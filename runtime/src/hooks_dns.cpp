// DNS View hooks (ticket 08). Virtualize *read* of DNS config only:
// GetNetworkParams / GetAdaptersAddresses. No UDP/TCP 53, DoH, WFP, or proxy.
// Host mode and Fail Open leave the original results untouched.
// When VirtualView is active both APIs see Profile servers (never a Host/Profile
// split). GetNetworkParams is IPv4-text only (IP_ADDR_STRING.String[16]); if
// Profile has no IPv4 servers the list is empty (still not Host).

#include <winsock2.h>

#include <ws2tcpip.h>

#include <iphlpapi.h>

#include <stdio.h>
#include <string.h>

#include "hooks.h"
#include "runtime_profile.h"

#include "audit.h"

static DWORD(WINAPI* TrueGetNetworkParams)(PFIXED_INFO, PULONG) =
    GetNetworkParams;
static ULONG(WINAPI* TrueGetAdaptersAddresses)(ULONG, ULONG, PVOID,
                                               PIP_ADAPTER_ADDRESSES, PULONG) =
    GetAdaptersAddresses;

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

    in_addr6 a6;
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

int EnvBoxInstallDnsHooks() {
  BuildDnsView();
  int ok = 0;
  ok += EnvBoxAttach(&TrueGetNetworkParams, HookGetNetworkParams);
  ok += EnvBoxAttach(&TrueGetAdaptersAddresses, HookGetAdaptersAddresses);
  return ok;
}
