#pragma once
#include "dns_transport.h"

// Explicit bootstrap addresses only; the URL identity is never resolved.
int DnsDohExchange(const DnsTransportEndpoint& endpoint,
                   const unsigned char* query, int query_length,
                   unsigned char* response, int capacity,
                   ULONGLONG deadline, HANDLE cancel_event);
