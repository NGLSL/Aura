#pragma once
#include <winsock2.h>
#include <windows.h>

// Transport consumes DNS wire packets and an absolute deadline. It has no
// QTYPE, record, cache, or Windows resolver semantics and never resolves IPs.
enum class DnsTransportKind { Udp, Tcp, Dot };
struct DnsTransportEndpoint {
  DnsTransportKind kind;
  const char* address;
  unsigned short port;
  const char* server_name = nullptr; // DoT TLS identity; never resolved.
};

// >0 = complete packet length, 0 = transport failure, -1 = cancellation.
// The caller owns buffers and endpoint strings for the duration of the call.
int DnsTransportExchange(const DnsTransportEndpoint& endpoint,
                         const unsigned char* query, int query_length,
                         unsigned char* response, int capacity,
                         ULONGLONG deadline, HANDLE cancel_event);
