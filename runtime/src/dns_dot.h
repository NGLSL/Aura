#pragma once
#include <winsock2.h>
#include <windows.h>

// Errors distinguish unavailable local revocation material from a revoked or
// otherwise invalid certificate. Every category is a failed upstream attempt.
enum class DnsDotError {
  None, InvalidArgument, Cancelled, Deadline, Network, Tls,
  Certificate, RevocationUnavailable, Revoked, Identity, Packet, Memory
};

// One bounded connection per query. No system name resolution, URL retrieval,
// certificate-store mutation, implicit transport fallback, or connection pool.
// >0 DNS packet bytes; 0 failure; -1 cancellation. error may be null.
int DnsDotExchange(const char* literal_ip, unsigned short port,
                   const char* server_name, const unsigned char* query,
                   int query_length, unsigned char* response, int capacity,
                   ULONGLONG deadline, HANDLE cancel_event,
                   DnsDotError* error = nullptr);

#ifdef ENVBOX_DNS_TRANSPORT_TESTING
#include <wincrypt.h>
// Only a standalone fixture compiles this seam. No product runtime export,
// environment switch, or mutable global test trust exists.
int DnsDotExchangeForTest(const char* literal_ip, unsigned short port,
                          const char* server_name, const unsigned char* query,
                          int query_length, unsigned char* response, int capacity,
                          ULONGLONG deadline, HANDLE cancel_event,
                          HCERTCHAINENGINE chain_engine, DnsDotError* error);
#endif
