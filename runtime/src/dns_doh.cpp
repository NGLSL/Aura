#include "dns_doh.h"
#include "envbox_dns_doh.h"
#include "audit.h"
#include <ws2tcpip.h>
#include <cstring>

namespace {
int32_t ENVBOX_DOH_CALL Cancelled(void* context) {
  HANDLE event = static_cast<HANDLE>(context);
  return event && WaitForSingleObject(event, 0) == WAIT_OBJECT_0 ? 1 : 0;
}

bool LiteralAuthority(const char* url, char (&ip)[64]) {
  if (!url || strncmp(url, "https://", 8) != 0) return false;
  const char* host = url + 8;
  const char* end;
  if (*host == '[') {
    ++host;
    end = strchr(host, ']');
    if (!end || (end[1] && end[1] != ':' && end[1] != '/' && end[1] != '?')) return false;
  } else {
    end = host + strcspn(host, ":/?");
  }
  size_t length = static_cast<size_t>(end - host);
  if (!length || length >= sizeof(ip)) return false;
  memcpy(ip, host, length);
  ip[length] = '\0';
  in_addr v4;
  in6_addr v6;
  return InetPtonA(AF_INET, ip, &v4) == 1 || InetPtonA(AF_INET6, ip, &v6) == 1;
}

const char* Summary(uint32_t error) {
  switch (error) {
    case EnvBoxDohNone: return "doh-success";
    case EnvBoxDohArgument: return "doh-invalid-configuration";
    case EnvBoxDohCancelledError: return "doh-cancelled";
    case EnvBoxDohDeadline: return "doh-deadline";
    case EnvBoxDohNetwork: return "doh-network-error";
    case EnvBoxDohTls: return "doh-tls-error";
    case EnvBoxDohCertificate: return "doh-certificate-invalid";
    case EnvBoxDohRevocationUnknown: return "doh-revocation-unavailable";
    case EnvBoxDohRevoked: return "doh-certificate-revoked";
    case EnvBoxDohIdentity: return "doh-identity-mismatch";
    case EnvBoxDohDisallowed: return "doh-certificate-disallowed";
    case EnvBoxDohTrustSnapshot: return "doh-trust-snapshot-error";
    case EnvBoxDohHttpStatus: return "doh-http-status-error";
    case EnvBoxDohMediaType: return "doh-media-type-error";
    case EnvBoxDohBodyLimit: return "doh-body-limit";
    case EnvBoxDohHttp: return "doh-http-error";
    case EnvBoxDohPanic: return "doh-internal-error";
    case EnvBoxDohContentEncoding: return "doh-content-encoding-error";
    default: return "doh-unknown-error";
  }
}
}

int DnsDohExchange(const DnsTransportEndpoint& endpoint,
                   const unsigned char* query, int query_length,
                   unsigned char* response, int capacity,
                   ULONGLONG deadline, HANDLE cancel_event) {
  if (!endpoint.url || endpoint.bootstrap_count < 0 || endpoint.bootstrap_count > 8 ||
      (endpoint.bootstrap_count && !endpoint.bootstrap_ips) || endpoint.tls_revocation > 1) return 0;
  char literal[64] = {};
  if (!endpoint.bootstrap_count && !LiteralAuthority(endpoint.url, literal)) {
    EnvBoxAuditEvent("DnsTransport.DoH", 1, "doh-bootstrap-required");
    return 0;
  }
  int count = endpoint.bootstrap_count ? endpoint.bootstrap_count : 1;
  for (int index = 0; index < count; ++index) {
    if (Cancelled(cancel_event)) return -1;
    if (GetTickCount64() >= deadline) return 0;
    const char* ip = endpoint.bootstrap_count ? endpoint.bootstrap_ips[index] : literal;
    uint32_t error = EnvBoxDohNone;
    int32_t result = envbox_doh_query_with_policy(
        reinterpret_cast<const uint8_t*>(endpoint.url), strlen(endpoint.url),
        reinterpret_cast<const uint8_t*>(ip), strlen(ip), query, query_length,
        response, capacity, deadline, Cancelled, cancel_event,
        endpoint.tls_revocation, &error);
    EnvBoxAuditEvent("DnsTransport.DoH", 1, Summary(error));
    if (result != 0) return result;
    if (error == EnvBoxDohCancelledError) return -1;
    if (error == EnvBoxDohDeadline) return 0;
  }
  return 0;
}
