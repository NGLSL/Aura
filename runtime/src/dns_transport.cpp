#include "dns_transport.h"
#include "dns_dot.h"
#include "audit.h"
#include <ws2tcpip.h>

namespace {
struct SocketOwner {
  SOCKET value;
  ~SocketOwner() { if (value != INVALID_SOCKET) closesocket(value); }
};

int Ready(SOCKET socket, bool writing, ULONGLONG deadline, HANDLE cancel) {
  for (;;) {
    if (cancel && WaitForSingleObject(cancel, 0) == WAIT_OBJECT_0) return -1;
    ULONGLONG now = GetTickCount64();
    if (now >= deadline) return 0;
    DWORD slice = static_cast<DWORD>((deadline - now) < 50 ? deadline - now : 50);
    fd_set set;
    FD_ZERO(&set);
    FD_SET(socket, &set);
    timeval timeout = {0, static_cast<long>(slice) * 1000};
    int result = select(0, writing ? nullptr : &set, writing ? &set : nullptr,
                        nullptr, &timeout);
    if (result > 0) {
      if (cancel && WaitForSingleObject(cancel, 0) == WAIT_OBJECT_0) return -1;
      return GetTickCount64() < deadline ? 1 : 0;
    }
    if (result == SOCKET_ERROR) return 0;
  }
}

int Transfer(SOCKET socket, unsigned char* buffer, int length, bool writing,
             ULONGLONG deadline, HANDLE cancel) {
  int offset = 0;
  while (offset < length) {
    int result = Ready(socket, writing, deadline, cancel);
    if (result <= 0) return result;
    int count = writing ? send(socket, reinterpret_cast<char*>(buffer + offset), length - offset, 0)
                        : recv(socket, reinterpret_cast<char*>(buffer + offset), length - offset, 0);
    if (count == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK) continue;
    if (count <= 0) return 0;
    offset += count;
  }
  return 1;
}
}

int DnsTransportExchange(const DnsTransportEndpoint& endpoint,
                         const unsigned char* query, int query_length,
                         unsigned char* response, int capacity,
                         ULONGLONG deadline, HANDLE cancel_event) {
  if (!endpoint.address || !endpoint.port || !query || !response ||
      query_length < 12 || query_length > 65535 || capacity < 12) return 0;
  if (endpoint.kind == DnsTransportKind::Dot) {
    DnsDotError error = DnsDotError::None;
    int result = DnsDotExchange(endpoint.address, endpoint.port, endpoint.server_name,
                                query, query_length, response, capacity,
                                deadline, cancel_event, &error);
    const char* summary = "dot-success";
    switch (error) {
      case DnsDotError::None: break;
      case DnsDotError::RevocationUnavailable: summary = "dot-revocation-unavailable"; break;
      case DnsDotError::Revoked: summary = "dot-certificate-revoked"; break;
      case DnsDotError::Certificate: summary = "dot-certificate-invalid"; break;
      case DnsDotError::Identity: summary = "dot-identity-mismatch"; break;
      case DnsDotError::Cancelled: summary = "dot-cancelled"; break;
      case DnsDotError::Deadline: summary = "dot-deadline"; break;
      case DnsDotError::Network: summary = "dot-network-error"; break;
      case DnsDotError::Tls: summary = "dot-tls-error"; break;
      case DnsDotError::Packet: summary = "dot-packet-error"; break;
      case DnsDotError::Memory: summary = "dot-allocation-error"; break;
      case DnsDotError::InvalidArgument: summary = "dot-invalid-configuration"; break;
    }
    EnvBoxAuditEvent("DnsTransport.DoT", 1, summary);
    return result;
  }
  if (endpoint.kind != DnsTransportKind::Udp && endpoint.kind != DnsTransportKind::Tcp) return 0;
  if (cancel_event && WaitForSingleObject(cancel_event, 0) == WAIT_OBJECT_0) return -1;
  if (GetTickCount64() >= deadline) return 0;
  sockaddr_storage address = {};
  auto v4 = reinterpret_cast<sockaddr_in*>(&address);
  auto v6 = reinterpret_cast<sockaddr_in6*>(&address);
  int address_length;
  if (InetPtonA(AF_INET, endpoint.address, &v4->sin_addr) == 1) {
    v4->sin_family = AF_INET;
    v4->sin_port = htons(endpoint.port);
    address_length = sizeof(*v4);
  } else if (InetPtonA(AF_INET6, endpoint.address, &v6->sin6_addr) == 1) {
    v6->sin6_family = AF_INET6;
    v6->sin6_port = htons(endpoint.port);
    address_length = sizeof(*v6);
  } else return 0;
  bool tcp = endpoint.kind == DnsTransportKind::Tcp;
  SocketOwner socket = {::socket(address.ss_family, tcp ? SOCK_STREAM : SOCK_DGRAM,
                                 tcp ? IPPROTO_TCP : IPPROTO_UDP)};
  if (socket.value == INVALID_SOCKET) return 0;
  u_long nonblocking = 1;
  if (ioctlsocket(socket.value, FIONBIO, &nonblocking) != 0) return 0;
  int result = connect(socket.value, reinterpret_cast<sockaddr*>(&address), address_length);
  if (result != 0) {
    int error = WSAGetLastError();
    if (error != WSAEWOULDBLOCK && error != WSAEINPROGRESS) return 0;
    result = Ready(socket.value, true, deadline, cancel_event);
    if (result <= 0) return result;
    int length = sizeof(error);
    if (getsockopt(socket.value, SOL_SOCKET, SO_ERROR,
                   reinterpret_cast<char*>(&error), &length) != 0 || error != 0) return 0;
  }
  if (tcp) {
    unsigned char prefix[2] = {static_cast<unsigned char>(query_length >> 8),
                               static_cast<unsigned char>(query_length)};
    result = Transfer(socket.value, prefix, 2, true, deadline, cancel_event);
    if (result <= 0) return result;
    result = Transfer(socket.value, const_cast<unsigned char*>(query), query_length,
                      true, deadline, cancel_event);
    if (result <= 0) return result;
    result = Transfer(socket.value, prefix, 2, false, deadline, cancel_event);
    if (result <= 0) return result;
    int length = (static_cast<int>(prefix[0]) << 8) | prefix[1];
    if (length < 12 || length > capacity) return 0;
    result = Transfer(socket.value, response, length, false, deadline, cancel_event);
    return result <= 0 ? result : length;
  }
  // Connected UDP restricts received packets to this literal IP and port.
  for (;;) {
    result = Ready(socket.value, true, deadline, cancel_event);
    if (result <= 0) return result;
    int sent = send(socket.value, reinterpret_cast<const char*>(query), query_length, 0);
    if (sent == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK) continue;
    if (sent != query_length) return 0;
    break;
  }
  for (;;) {
    result = Ready(socket.value, false, deadline, cancel_event);
    if (result <= 0) return result;
    int received = recv(socket.value, reinterpret_cast<char*>(response), capacity, 0);
    if (received == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK) continue;
    return received > 0 ? received : 0;
  }
}
