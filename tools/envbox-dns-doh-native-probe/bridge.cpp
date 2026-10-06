#include <windows.h>
#include <stdint.h>
#include <string.h>
#include "envbox_dns_doh.h"

// This DLL deliberately exposes only a small probe entry point.  The Rust
// staticlib is the product-default build: no fixture trust feature or test
// snapshot is linked into this process.
BOOL WINAPI DllMain(HINSTANCE, DWORD, LPVOID) { return TRUE; }

struct ProbeOutcome {
  uint32_t policy;
  uint32_t error;
  int32_t length;
  uint16_t query_id;
  uint16_t response_id;
  uint8_t qr;
  uint8_t rcode;
  uint8_t question_match;
  uint8_t response_shape;
};

static uint16_t ReadU16(const uint8_t* data) {
  return static_cast<uint16_t>((static_cast<uint16_t>(data[0]) << 8) | data[1]);
}

static bool SameDnsLabel(const uint8_t* actual, const uint8_t* expected, uint8_t length) {
  for (uint8_t index = 0; index < length; ++index) {
    const uint8_t lhs = actual[index];
    const uint8_t rhs = expected[index];
    const uint8_t lhs_lower = (lhs >= 'A' && lhs <= 'Z') ? static_cast<uint8_t>(lhs + ('a' - 'A')) : lhs;
    const uint8_t rhs_lower = (rhs >= 'A' && rhs <= 'Z') ? static_cast<uint8_t>(rhs + ('a' - 'A')) : rhs;
    if (lhs_lower != rhs_lower) return false;
  }
  return true;
}

static bool QuestionMatches(const uint8_t* response, int32_t length) {
  if (length < 12 || ReadU16(response + 4) != 1) return false;
  const uint8_t expected_qname[] = {7, 'e', 'x', 'a', 'm', 'p', 'l', 'e', 3, 'c', 'o', 'm', 0};
  size_t offset = 12;
  size_t expected_offset = 0;
  while (expected_offset < sizeof(expected_qname)) {
    if (offset >= static_cast<size_t>(length)) return false;
    const uint8_t label_length = response[offset++];
    if (label_length == 0) {
      if (expected_qname[expected_offset] != 0) return false;
      ++expected_offset;
      break;
    }
    if ((label_length & 0xc0) != 0 || label_length > 63 ||
        offset + label_length > static_cast<size_t>(length) ||
        expected_offset >= sizeof(expected_qname) ||
        expected_qname[expected_offset] != label_length ||
        expected_offset + 1 + label_length > sizeof(expected_qname) ||
        !SameDnsLabel(response + offset, expected_qname + expected_offset + 1, label_length)) {
      return false;
    }
    offset += label_length;
    expected_offset += 1 + label_length;
  }
  if (expected_offset != sizeof(expected_qname) || offset + 4 > static_cast<size_t>(length)) return false;
  return ReadU16(response + offset) == 1 && ReadU16(response + offset + 2) == 1;
}

static void MakeQuery(uint8_t* packet, uint16_t id) {
  memset(packet, 0, 12);
  packet[0] = static_cast<uint8_t>(id >> 8);
  packet[1] = static_cast<uint8_t>(id);
  packet[2] = 0x01; // RD
  packet[5] = 0x01; // QDCOUNT
  const uint8_t qname[] = {7, 'e', 'x', 'a', 'm', 'p', 'l', 'e', 3, 'c', 'o', 'm', 0};
  memcpy(packet + 12, qname, sizeof(qname));
  const size_t offset = 12 + sizeof(qname);
  packet[offset + 0] = 0;
  packet[offset + 1] = 1; // A
  packet[offset + 2] = 0;
  packet[offset + 3] = 1; // IN
}

static int32_t Query(const char* url, const char* ip, uint32_t policy, ProbeOutcome* outcome) {
  uint8_t query[64] = {};
  uint8_t response[65535] = {};
  const uint16_t id = 0xA042;
  MakeQuery(query, id);
  uint32_t error = 0;
  const int32_t length = envbox_doh_query_with_policy(
      reinterpret_cast<const uint8_t*>(url), strlen(url),
      reinterpret_cast<const uint8_t*>(ip), strlen(ip), query,
      12 + 13 + 4, response, sizeof(response), GetTickCount64() + 15000,
      nullptr, nullptr, policy, &error);
  outcome->policy = policy;
  outcome->error = error;
  outcome->length = length;
  outcome->query_id = id;
  outcome->response_id = 0;
  outcome->qr = 0;
  outcome->rcode = 0;
  outcome->question_match = 0;
  outcome->response_shape = 0;
  if (length < 12) return length;
  outcome->response_id = ReadU16(response);
  outcome->qr = static_cast<uint8_t>((response[2] >> 7) & 1);
  outcome->rcode = static_cast<uint8_t>(response[3] & 0x0f);
  // Compare the complete wire question, not only the message ID and QDCOUNT:
  // QNAME is case-insensitive on the wire, while QTYPE and QCLASS must match
  // the query sent above exactly.
  outcome->question_match = static_cast<uint8_t>(
      outcome->response_id == id && QuestionMatches(response, length));
  outcome->response_shape = static_cast<uint8_t>(outcome->qr == 1 && outcome->question_match);
  return length;
}

extern "C" __declspec(dllexport) int __cdecl RunNativeDohProbe(uint32_t index, ProbeOutcome* outcome) {
  if (!outcome || index >= 7) return 2;
  // One endpoint per process keeps the process-local API trap's allowance
  // fixed before trust loading and network activity.
  const char* urls[] = {
      "https://cloudflare-dns.com/dns-query", "https://cloudflare-dns.com/dns-query",
      "https://dns.google/dns-query", "https://dns.google/dns-query",
      "https://cloudflare-dns.com/dns-query", "https://dns.google/dns-query",
      "https://cloudflare-dns.com/dns-query"};
  const char* ips[] = {"1.1.1.1", "2606:4700:4700::1111", "8.8.8.8",
      "2001:4860:4860::8888", "1.1.1.1", "8.8.8.8", "1.1.1.1"};
  const uint32_t policies[] = {0, 0, 0, 0, 1, 1, 2};
  Query(urls[index], ips[index], policies[index], outcome);
  return 0;
}
