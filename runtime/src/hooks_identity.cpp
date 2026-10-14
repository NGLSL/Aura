// Winsock2 must precede hooks.h/windows.h; NetIO needs ws2tcpip first.
#include <winsock2.h>
#include <ws2tcpip.h>
#include <iphlpapi.h>
#include <netioapi.h>
#include <cstring>
#include <cstdio>
#include "hooks.h"
#include "audit.h"

static auto TrueGetComputerNameW = &GetComputerNameW;
static auto TrueGetComputerNameA = &GetComputerNameA;
static auto TrueGetComputerNameExW = &GetComputerNameExW;
static auto TrueGetComputerNameExA = &GetComputerNameExA;
static auto TrueGetHostNameW = &GetHostNameW;
static auto TrueGetHostNameA = &gethostname;
static auto TrueGetUserNameW = &GetUserNameW;
static auto TrueGetUserNameA = &GetUserNameA;
static auto TrueGetAdaptersInfo = &GetAdaptersInfo;
static auto TrueGetIfEntry = &GetIfEntry;
static auto TrueGetIfTable = &GetIfTable;
static auto TrueGetIfEntry2 = &GetIfEntry2;
static auto TrueGetIfTable2 = &GetIfTable2;
template <class C>
static BOOL Name(const wchar_t *text, C *out, LPDWORD size, DWORD short_error,
                 bool user) {
  if (!size) {
    SetLastError(ERROR_INVALID_PARAMETER);
    return FALSE;
  }
  DWORD n = (DWORD)wcslen(text);
  if (!out || *size < n + 1) {
    *size = n + 1;
    SetLastError(short_error);
    return FALSE;
  }
  for (DWORD i = 0; i <= n; ++i) out[i] = (C)text[i];
  *size = n + (user ? 1 : 0);
  return TRUE;
}
static const wchar_t *Computer(COMPUTER_NAME_FORMAT format) {
  const auto *p = EnvBoxProfile();
  if (!p->identity_computer_name[0]) return nullptr;
  switch (format) {
    case ComputerNameDnsDomain:
    case ComputerNamePhysicalDnsDomain:
      return L"";
    case ComputerNameNetBIOS:
    case ComputerNameDnsHostname:
    case ComputerNameDnsFullyQualified:
    case ComputerNamePhysicalNetBIOS:
    case ComputerNamePhysicalDnsHostname:
    case ComputerNamePhysicalDnsFullyQualified:
      return p->identity_computer_name;
    default:
      return nullptr;
  }
}
static BOOL WINAPI HookGetComputerNameW(LPWSTR b, LPDWORD n) {
  const auto *s = Computer(ComputerNameNetBIOS);
  return s ? Name(s, b, n, ERROR_BUFFER_OVERFLOW, false)
           : TrueGetComputerNameW(b, n);
}
static BOOL WINAPI HookGetComputerNameA(LPSTR b, LPDWORD n) {
  const auto *s = Computer(ComputerNameNetBIOS);
  return s ? Name(s, b, n, ERROR_BUFFER_OVERFLOW, false)
           : TrueGetComputerNameA(b, n);
}
static BOOL WINAPI HookGetComputerNameExW(COMPUTER_NAME_FORMAT f, LPWSTR b,
                                          LPDWORD n) {
  const auto *s = Computer(f);
  return s ? Name(s, b, n, ERROR_MORE_DATA, false)
           : TrueGetComputerNameExW(f, b, n);
}
static BOOL WINAPI HookGetComputerNameExA(COMPUTER_NAME_FORMAT f, LPSTR b,
                                          LPDWORD n) {
  const auto *s = Computer(f);
  return s ? Name(s, b, n, ERROR_MORE_DATA, false)
           : TrueGetComputerNameExA(f, b, n);
}
static BOOL WINAPI HookGetUserNameW(LPWSTR b, LPDWORD n) {
  const auto *s = EnvBoxProfile()->identity_user_name;
  return s[0] ? Name(s, b, n, ERROR_INSUFFICIENT_BUFFER, true)
              : TrueGetUserNameW(b, n);
}
static BOOL WINAPI HookGetUserNameA(LPSTR b, LPDWORD n) {
  const auto *s = EnvBoxProfile()->identity_user_name;
  return s[0] ? Name(s, b, n, ERROR_INSUFFICIENT_BUFFER, true)
              : TrueGetUserNameA(b, n);
}
template<class C> static int HostName(C* buffer, int capacity) {
  // Check initialization without resolving a name or changing WSAStartup's count.
  const int previous = WSAGetLastError();
  sockaddr_storage address = {};
  int size = sizeof(address);
  getpeername(INVALID_SOCKET, reinterpret_cast<sockaddr*>(&address), &size);
  if (WSAGetLastError() == WSANOTINITIALISED) return SOCKET_ERROR;
  WSASetLastError(previous);
  const wchar_t* name = EnvBoxProfile()->identity_computer_name;
  const size_t length = wcslen(name);
  if (!buffer || capacity <= 0 || static_cast<size_t>(capacity) <= length) {
    WSASetLastError(WSAEFAULT);
    return SOCKET_ERROR;
  }
  for (size_t i = 0; i <= length; ++i) buffer[i] = static_cast<C>(name[i]);
  return 0;
}
static int WSAAPI HookGetHostNameW(PWSTR buffer,int capacity) {
  return EnvBoxProfile()->identity_computer_name[0] ? HostName(buffer,capacity) : TrueGetHostNameW(buffer,capacity);
}
static int WSAAPI HookGetHostNameA(char* buffer,int capacity) {
  return EnvBoxProfile()->identity_computer_name[0] ? HostName(buffer,capacity) : TrueGetHostNameA(buffer,capacity);
}
static bool Mac(BYTE *data, ULONG length) {
  const auto *p = EnvBoxProfile();
  if (p->identity_mac_address[0] && length == 6) {
    memcpy(data, p->identity_mac_bytes, 6);
    return true;
  }
  return false;
}
static void AuditMac(const char *api, DWORD status, bool covered,
                     DWORD last_error) {
  char summary[96];
  const char *view = status != NO_ERROR
      ? ((status == ERROR_BUFFER_OVERFLOW || status == ERROR_INSUFFICIENT_BUFFER)
             ? "mac-buffer-size-query" : "mac-api-error")
      : (covered ? "mac-virtual-view"
                 : (EnvBoxProfile()->identity_mac_address[0]
                        ? "mac-no-six-byte-address" : "mac-host"));
  snprintf(summary, sizeof(summary), "%s status=%lu", view,
           static_cast<unsigned long>(status));
  EnvBoxAuditEvent(api, covered ? 1 : 0, summary);
  // IP Helper returns a status code; preserve the thread error state as well.
  SetLastError(last_error);
}
static ULONG WINAPI HookGetAdaptersInfo(PIP_ADAPTER_INFO p, PULONG n) {
  ULONG s = TrueGetAdaptersInfo(p, n);
  const DWORD last_error = GetLastError();
  bool covered = false;
  if (s == NO_ERROR)
    for (auto *a = p; a; a = a->Next)
      covered = Mac(a->Address, a->AddressLength) || covered;
  AuditMac("GetAdaptersInfo", s, covered, last_error);
  return s;
}
static DWORD WINAPI HookGetIfEntry(PMIB_IFROW p) {
  DWORD s = TrueGetIfEntry(p);
  const DWORD last_error = GetLastError();
  const bool covered = s == NO_ERROR && Mac(p->bPhysAddr, p->dwPhysAddrLen);
  AuditMac("GetIfEntry", s, covered, last_error);
  return s;
}
static DWORD WINAPI HookGetIfTable(PMIB_IFTABLE p, PULONG n, BOOL order) {
  DWORD s = TrueGetIfTable(p, n, order);
  const DWORD last_error = GetLastError();
  bool covered = false;
  if (s == NO_ERROR && p)
    for (DWORD i = 0; i < p->dwNumEntries; ++i)
      covered = Mac(p->table[i].bPhysAddr, p->table[i].dwPhysAddrLen) || covered;
  AuditMac("GetIfTable", s, covered, last_error);
  return s;
}
static bool Row2(PMIB_IF_ROW2 p) {
  const bool covered = Mac(p->PhysicalAddress, p->PhysicalAddressLength);
  Mac(p->PermanentPhysicalAddress, p->PhysicalAddressLength);
  return covered;
}
static DWORD WINAPI HookGetIfEntry2(PMIB_IF_ROW2 p) {
  DWORD s = TrueGetIfEntry2(p);
  const DWORD last_error = GetLastError();
  const bool covered = s == NO_ERROR && Row2(p);
  AuditMac("GetIfEntry2", s, covered, last_error);
  return s;
}
static DWORD WINAPI HookGetIfTable2(PMIB_IF_TABLE2 *p) {
  DWORD s = TrueGetIfTable2(p);
  const DWORD last_error = GetLastError();
  bool covered = false;
  if (s == NO_ERROR && p && *p)
    for (ULONG i = 0; i < (*p)->NumEntries; ++i)
      covered = Row2(&(*p)->Table[i]) || covered;
  AuditMac("GetIfTable2", s, covered, last_error);
  return s;
}
int EnvBoxIdentityHooksRequired() {
  const auto *p = EnvBoxProfile();
  return (p->identity_computer_name[0] ? 6 : 0) +
         (p->identity_user_name[0] ? 2 : 0) +
         (p->identity_mac_address[0] ? 5 : 0);
}
int EnvBoxInstallIdentityHooks() {
  const auto *p = EnvBoxProfile();
  int n = 0;
  if (p->identity_computer_name[0]) {
    n += EnvBoxAttach(&TrueGetComputerNameW, HookGetComputerNameW);
    n += EnvBoxAttach(&TrueGetComputerNameA, HookGetComputerNameA);
    n += EnvBoxAttach(&TrueGetComputerNameExW, HookGetComputerNameExW);
    n += EnvBoxAttach(&TrueGetComputerNameExA, HookGetComputerNameExA);
    n += EnvBoxAttach(&TrueGetHostNameW, HookGetHostNameW);
    n += EnvBoxAttach(&TrueGetHostNameA, HookGetHostNameA);
  }
  if (p->identity_user_name[0]) {
    n += EnvBoxAttach(&TrueGetUserNameW, HookGetUserNameW);
    n += EnvBoxAttach(&TrueGetUserNameA, HookGetUserNameA);
  }
  if (p->identity_mac_address[0]) {
    n += EnvBoxAttach(&TrueGetAdaptersInfo, HookGetAdaptersInfo);
    n += EnvBoxAttach(&TrueGetIfEntry, HookGetIfEntry);
    n += EnvBoxAttach(&TrueGetIfTable, HookGetIfTable);
    n += EnvBoxAttach(&TrueGetIfEntry2, HookGetIfEntry2);
    n += EnvBoxAttach(&TrueGetIfTable2, HookGetIfTable2);
  }
  return n;
}
