#pragma once
#include "dns_transport.h"

// Returns a DNS authority needing Profile bootstrap, never a literal IP.
bool DnsDohBootstrapName(const DnsTransportEndpoint& endpoint, char (&name)[256]);

// Transport requires literal connection addresses; Profile routing supplies
// them without changing the URL's HTTP/TLS identity or invoking Host DNS.
int DnsDohExchange(const DnsTransportEndpoint& endpoint,
                   const unsigned char* query, int query_length,
                   unsigned char* response, int capacity,
                   ULONGLONG deadline, HANDLE cancel_event);
