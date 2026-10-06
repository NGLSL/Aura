// Native contract fixture: the actual Profile route, packet/question checks,
// DNS record decoder and DoH wrapper run against a deterministic transport seam.
// This does not claim to exercise TCP/UDP/SChannel/Rust TLS internals.
#include "../../runtime/src/hooks_dns.cpp"
#include "envbox_dns_doh.h"
#include <vector>
#include <string>

namespace {
RuntimeProfile profile;
int local_ex_calls = 0;
void Require(bool condition, const char* message);
DNS_STATUS WINAPI LocalNativeQuery(PDNS_QUERY_REQUEST request,
                                   PDNS_QUERY_RESULT result,
                                   PDNS_QUERY_CANCEL) {
  Require(request->Version == 2, "native private request remains opaque");
  ++local_ex_calls;
  result->QueryStatus = ERROR_SUCCESS;
  return ERROR_SUCCESS;
}
INT WSAAPI LocalNativeAddressW(PCWSTR, PCWSTR, const ADDRINFOW*, PADDRINFOW*) {
  DNS_QUERY_REQUEST request = {};
  request.Version = 2;
  request.QueryName = L"localhost";
  request.QueryType = DNS_TYPE_A;
  DNS_QUERY_RESULT result = {};
  return HookDnsQueryEx(&request, &result, nullptr) == ERROR_SUCCESS ? 0 : EAI_NONAME;
}
INT WSAAPI LocalNativeAddressA(PCSTR, PCSTR, const ADDRINFOA*, PADDRINFOA*) {
  return LocalNativeAddressW(nullptr, nullptr, nullptr, nullptr);
}
enum class Reply { Alias, InlineAlias, Unrelated, WrongId, WrongQuestion, Loop, Cancel, Timeout, Unspecified, Multicast, Broadcast };
Reply reply = Reply::Alias;
int seed_calls = 0, doh_calls = 0, host_calls = 0;
ULONGLONG accepted_deadline = 0;
HANDLE accepted_cancel = nullptr;
const char* kUrl = "https://doh.profile.test:8443/dns-query";

void Require(bool condition, const char* message) {
  if (!condition) { std::fprintf(stderr, "FAIL: %s\n", message); std::exit(1); }
}
void AddName(std::vector<unsigned char>& packet, const char* name) {
  unsigned char encoded[256];
  int count = EncodeDnsName(name, encoded, sizeof(encoded));
  Require(count > 0, "fixture name encoding");
  packet.insert(packet.end(), encoded, encoded + count);
}
void AddRecord(std::vector<unsigned char>& packet, const char* owner, unsigned type,
               const std::vector<unsigned char>& data) {
  AddName(packet, owner);
  unsigned char header[10] = {};
  WriteU16(header, type); WriteU16(header + 2, 1); header[7] = 10;
  WriteU16(header + 8, static_cast<unsigned>(data.size()));
  packet.insert(packet.end(), header, header + 10);
  packet.insert(packet.end(), data.begin(), data.end());
}
int Answer(const unsigned char* query, int length, unsigned char* output, int capacity, bool seed) {
  int offset = 12;
  char name[256];
  Require(DecodeDnsName(query, length, &offset, name, sizeof(name)) != 0, "fixture question decode");
  unsigned type = ReadU16(query + offset);
  std::vector<unsigned char> packet(query, query + length);
  WriteU16(packet.data() + 2, 0x8180);
  unsigned count = 1;
  if (seed) {
    Require(type == DNS_TYPE_A, "bootstrap only asks A");
    if (reply == Reply::Unspecified || reply == Reply::Multicast || reply == Reply::Broadcast) {
      AddRecord(packet, name, DNS_TYPE_A, reply == Reply::Unspecified ? std::vector<unsigned char>{0, 0, 0, 0}
          : reply == Reply::Multicast ? std::vector<unsigned char>{224, 0, 0, 1} : std::vector<unsigned char>{255, 255, 255, 255});
    } else if (reply == Reply::Unrelated) {
      AddRecord(packet, "unrelated.profile.test", DNS_TYPE_A, {6, 6, 6, 6});
    } else if (!strcmp(name, "doh.profile.test")) {
      std::vector<unsigned char> alias; AddName(alias, "alias.profile.test");
      AddRecord(packet, name, DNS_TYPE_CNAME, alias);
      AddRecord(packet, "unrelated.profile.test", DNS_TYPE_A, {6, 6, 6, 6});
      count = 2;
      if (reply == Reply::InlineAlias) {
        AddRecord(packet, "alias.profile.test", DNS_TYPE_A, {1, 1, 1, 1}); ++count;
      }
    } else if (reply == Reply::Loop) {
      std::vector<unsigned char> alias; AddName(alias, "doh.profile.test");
      AddRecord(packet, name, DNS_TYPE_CNAME, alias);
    } else {
      AddRecord(packet, name, DNS_TYPE_A, {1, 1, 1, 1});
    }
    if (reply == Reply::WrongId) packet[0] ^= 0x80;
    if (reply == Reply::WrongQuestion) packet[13] ^= 0x01;
  } else {
    AddRecord(packet, name, type, type == DNS_TYPE_A ? std::vector<unsigned char>{10, 99, 0, 1}
        : type == DNS_TYPE_AAAA ? std::vector<unsigned char>(16, 1) : std::vector<unsigned char>{0, 1, 0});
  }
  WriteU16(packet.data() + 6, count);
  Require(packet.size() <= static_cast<size_t>(capacity), "fixture packet capacity");
  memcpy(output, packet.data(), packet.size());
  return static_cast<int>(packet.size());
}
void Configure(RuntimeDnsUpstreamType kind, bool explicit_ip = false, bool literal = false) {
  profile = {};
  profile.dns_mode = 1; profile.dns_strict = 1; profile.dns_config_version = 1;
  profile.dns_upstream_count = 2;
  auto& target = profile.dns_upstreams[0]; target.type = EnvBoxDnsDoh;
  strcpy_s(target.url, kUrl); target.tls_revocation = 1;
  auto& seed = profile.dns_upstreams[1]; seed.type = kind;
  strcpy_s(seed.address, "127.0.0.1"); seed.port = 5353;
  strcpy_s(seed.server_name, "seed.profile.test");
  if (kind == EnvBoxDnsDoh) {
    strcpy_s(seed.url, literal ? "https://127.0.0.1/dns-query" : "https://seed.profile.test/dns-query");
    if (explicit_ip) { seed.bootstrap_count = 1; strcpy_s(seed.bootstrap_ips[0], "127.0.0.1"); }
  }
  reply = Reply::Alias;
  seed_calls = doh_calls = host_calls = 0;
  accepted_deadline = GetTickCount64() + 1000;
  accepted_cancel = nullptr;
  memset(g_dns_cache, 0, sizeof(g_dns_cache));
  g_view_active = 1;
}
int Query(unsigned type = DNS_TYPE_HTTPS) {
  DnsTransportEndpoint target;
  Require(DnsEndpoint(&profile, 0, &target) == 1, "target endpoint");
  PDNS_RECORD records = nullptr;
  DNS_STATUS status = ERROR_TIMEOUT;
  int result = DnsQueryOne(target, "original.fixture.test", type, accepted_deadline,
                            accepted_cancel, nullptr, &records, &status);
  if (records) TrueDnsFree(records, DnsFreeRecordList);
  if (result > 0) Require(status == ERROR_SUCCESS, "original response decode");
  return result;
}
DNS_STATUS WINAPI HostQuery(PCWSTR, WORD, DWORD, PVOID, PDNS_RECORD*, PVOID*) {
  ++host_calls; return ERROR_ACCESS_DENIED;
}
INT WSAAPI HostAddress(PCSTR, PCSTR, const ADDRINFOA*, PADDRINFOA*) {
  ++host_calls; return EAI_FAIL;
}
}

const RuntimeProfile* EnvBoxProfile() { return &profile; }
void EnvBoxAuditEvent(const char*, int, const char* summary) {
  if (!strcmp(summary, "dns-host")) ++host_calls;
}
int32_t ENVBOX_DOH_CALL envbox_doh_query_with_policy(
    const uint8_t* url, size_t url_length, const uint8_t* ip, size_t ip_length,
    const uint8_t* query, size_t length, uint8_t* output, size_t capacity,
    uint64_t deadline, EnvBoxDohCancelled cancelled, void* context,
    uint32_t policy, uint32_t* error) {
  Require(deadline <= accepted_deadline, "original shared deadline");
  if (cancelled(context)) { *error = EnvBoxDohCancelledError; return -1; }
  if (GetTickCount64() >= deadline) { *error = EnvBoxDohDeadline; return 0; }
  bool seed = std::string(reinterpret_cast<const char*>(url), url_length) != kUrl;
  if (!seed) {
    ++doh_calls;
    Require(std::string(reinterpret_cast<const char*>(ip), ip_length) == "1.1.1.1", "CNAME-derived IP, not unrelated A");
    Require(policy == 1, "TLS revocation policy retained");
  } else { ++seed_calls; }
  *error = EnvBoxDohNone;
  return Answer(query, static_cast<int>(length), output, static_cast<int>(capacity), seed);
}
int DnsTransportExchange(const DnsTransportEndpoint& endpoint, const unsigned char* query,
                         int length, unsigned char* output, int capacity,
                         ULONGLONG deadline, HANDLE cancel) {
  Require(deadline <= accepted_deadline, "bootstrap shares original deadline");
  if (cancel && WaitForSingleObject(cancel, 0) == WAIT_OBJECT_0) return -1;
  if (GetTickCount64() >= deadline) return 0;
  if (endpoint.kind == DnsTransportKind::Doh) return DnsDohExchange(endpoint, query, length, output, capacity, deadline, cancel);
  ++seed_calls;
  if (reply == Reply::Cancel) { SetEvent(cancel); return -1; }
  if (reply == Reply::Timeout) { Sleep(20); return 0; }
  return Answer(query, length, output, capacity, true);
}

int main() {
  DnsWinsockScope winsock;
  Require(winsock.active != 0, "Winsock initialization");
  TrueDnsQuery_W = HostQuery; Truegetaddrinfo = HostAddress;
  Configure(EnvBoxDnsUdp);
  auto native_ex = TrueDnsQueryEx;
  TrueDnsQueryEx = LocalNativeQuery;
  DNS_QUERY_REQUEST local_request = {};
  local_request.Version = 2;
  local_request.QueryName = L"localhost";
  local_request.QueryType = DNS_TYPE_A;
  DNS_QUERY_RESULT local_result = {};
  {
    LocalhostResolutionScope local(L"LOCALHOST.");
    Require(HookDnsQueryEx(&local_request, &local_result, nullptr) == ERROR_SUCCESS &&
            local_ex_calls == 1, "private native localhost query preserved in local scope");
  }
  Require(g_localhost_resolution_depth == 0, "local resolver scope released");
  auto native_w = TrueGetAddrInfoW;
  auto native_a = Truegetaddrinfo;
  TrueGetAddrInfoW = LocalNativeAddressW;
  Truegetaddrinfo = LocalNativeAddressA;
  ADDRINFOW local_hints_w = {};
  local_hints_w.ai_flags = AI_PASSIVE;
  local_hints_w.ai_family = AF_UNSPEC;
  local_hints_w.ai_socktype = SOCK_STREAM;
  PADDRINFOW local_address_w = nullptr;
  Require(HookGetAddrInfoW(L"localhost", L"0", &local_hints_w, &local_address_w) == 0,
          "GetAddrInfoW preserves native localhost nested API with passive flag");
  ADDRINFOA local_hints_a = {};
  local_hints_a.ai_flags = AI_PASSIVE;
  local_hints_a.ai_family = AF_INET;
  PADDRINFOA local_address_a = nullptr;
  Require(Hookgetaddrinfo("localhost.", "0", &local_hints_a, &local_address_a) == 0,
          "getaddrinfo preserves native localhost nested API");
  Require(local_ex_calls == 3 && g_localhost_resolution_depth == 0,
          "native local API reentry is balanced and bounded");
  TrueGetAddrInfoW = native_w;
  Truegetaddrinfo = native_a;
  Require(HookDnsQueryEx(&local_request, &local_result, nullptr) == ERROR_NOT_SUPPORTED &&
          local_ex_calls == 3, "direct unsupported request has no native reentry permission");
  local_request.QueryName = L"real-domain.fixture.test";
  Require(HookDnsQueryEx(&local_request, &local_result, nullptr) == ERROR_NOT_SUPPORTED &&
          local_ex_calls == 3, "unsupported real domain never falls back to Host");
  {
    LocalhostResolutionScope domain("real-domain.fixture.test");
    Require(g_localhost_resolution_depth == 0, "real domain cannot grant native scope");
  }
  // A public request remains Profile-routed even if it reenters on the
  // thread currently completing Windows' local-name resolver operation.
  DNS_QUERY_REQUEST public_request = {};
  public_request.Version = DNS_QUERY_REQUEST_VERSION1;
  public_request.QueryName = L"public-domain.fixture.test";
  public_request.QueryType = DNS_TYPE_A;
  DNS_QUERY_RESULT public_result = {};
  public_result.Version = DNS_QUERY_RESULTS_VERSION1;
  accepted_deadline = GetTickCount64() + kDnsTotalBudgetMs;
  {
    LocalhostResolutionScope local(L"localhost");
    Require(HookDnsQueryEx(&public_request, &public_result, nullptr) == ERROR_SUCCESS &&
            public_result.pQueryRecords != nullptr && local_ex_calls == 3 && !host_calls,
            "public real-domain request remains Profile-routed inside local scope");
    Require(public_result.pQueryRecords->Data.A.IpAddress == inet_addr("10.99.0.1"),
            "nested public request uses Profile answer");
  }
  TrueDnsFree(public_result.pQueryRecords, DnsFreeRecordList);
  TrueDnsQueryEx = native_ex;
  for (auto kind : {EnvBoxDnsUdp, EnvBoxDnsTcp, EnvBoxDnsDot, EnvBoxDnsDoh}) {
    Configure(kind, kind == EnvBoxDnsDoh);
    Require(Query() == 1 && seed_calls == 2 && doh_calls == 1 && !host_calls, "supported seed and CNAME chain");
  }
  Configure(EnvBoxDnsDoh, false, true);
  Require(Query() == 1 && seed_calls == 2 && doh_calls == 1, "literal-host DoH seed");
  Configure(EnvBoxDnsUdp); reply = Reply::InlineAlias;
  Require(Query() == 1 && seed_calls == 1 && doh_calls == 1, "inline CNAME answer");
  for (auto invalid : {Reply::Unrelated, Reply::WrongId, Reply::WrongQuestion, Reply::Loop,
                      Reply::Unspecified, Reply::Multicast, Reply::Broadcast}) {
    Configure(EnvBoxDnsUdp); reply = invalid;
    Require(Query() == 0 && doh_calls == 0 && !host_calls, "invalid bootstrap refuses DoH and Host fallback");
  }
  Configure(EnvBoxDnsDoh);
  Require(Query() == 0 && seed_calls == 0 && doh_calls == 0 && !host_calls, "mutually unresolved DoH never recurses");
  Configure(EnvBoxDnsUdp);
  PDNS_RECORD records = nullptr;
  Require(RouteDnsQuery("rr.route.test", DNS_TYPE_HTTPS, 0, &records, nullptr,
      kDnsFlavorW, L"rr.route.test", "fixture", nullptr, accepted_deadline) == ERROR_SUCCESS, "full-QTYPE route");
  TrueDnsFree(records, DnsFreeRecordList);
  Configure(EnvBoxDnsUdp);
  ADDRINFOA hints = {}; hints.ai_family = AF_INET;
  PADDRINFOA addresses = nullptr;
  accepted_deadline = GetTickCount64() + kDnsTotalBudgetMs;
  Require(Hookgetaddrinfo("address.route.test", nullptr, &hints, &addresses) == 0 && addresses &&
      reinterpret_cast<sockaddr_in*>(addresses->ai_addr)->sin_addr.S_un.S_un_b.s_b1 == 10 && !host_calls,
      "getaddrinfo Profile route");
  Hookfreeaddrinfo(addresses);
  Configure(EnvBoxDnsUdp); reply = Reply::Cancel;
  accepted_cancel = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  Require(accepted_cancel != nullptr && Query() == -1 && doh_calls == 0 && !host_calls, "bootstrap cancellation");
  CloseHandle(accepted_cancel);
  Configure(EnvBoxDnsUdp); reply = Reply::Timeout;
  accepted_deadline = GetTickCount64() + 5;
  Require(Query() == 0 && doh_calls == 0 && !host_calls, "bootstrap deadline has no fresh budget");
  std::printf("Profile bootstrap native contracts passed; Host resolver calls=0.\n");
  return 0;
}
