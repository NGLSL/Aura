// Native contract fixture: the actual Profile route, packet/question checks,
// DNS record decoder and DoH wrapper run against a deterministic transport seam.
// This does not claim to exercise TCP/UDP/SChannel/Rust TLS internals.
#include "../../runtime/src/hooks_dns.cpp"
#include "envbox_dns_doh.h"
#include <vector>
#include <string>
#include <atomic>
#include <thread>
#include <chrono>

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
std::atomic<int> seed_calls{0}, doh_calls{0}, host_calls{0};
std::atomic<int> transport_delay_ms{0};
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
  transport_delay_ms = 0;
  accepted_deadline = GetTickCount64() + 1000;
  accepted_cancel = nullptr;
  memset(g_dns_cache, 0, sizeof(g_dns_cache));
#ifdef ENVBOX_DNS_RESPONSE_CACHE_FIXTURE
  EnvBoxDnsResponseCache::Reset();
#endif
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
    if (transport_delay_ms.load() > 0) Sleep(static_cast<DWORD>(transport_delay_ms.load()));
    Require(std::string(reinterpret_cast<const char*>(ip), ip_length) == "1.1.1.1", "CNAME-derived IP, not unrelated A");
    Require(policy == profile.dns_upstreams[0].tls_revocation, "TLS revocation policy retained");
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

namespace {
int legacy_extract_calls = 0;
DNS_STATUS WINAPI LegacyOpaqueExtractor(PDNS_MESSAGE_BUFFER message, WORD length,
                                       PDNS_RECORD* records) {
  ++legacy_extract_calls;
  std::vector<unsigned char> wire(reinterpret_cast<unsigned char*>(message),
                                  reinterpret_cast<unsigned char*>(message) + length);
  DNS_BYTE_FLIP_HEADER_COUNTS(&reinterpret_cast<PDNS_MESSAGE_BUFFER>(wire.data())->MessageHead);
  unsigned mapped = 0;
  bool has_binding = false;
  if (ScanDnsOpaqueWireRecords(wire.data(), length, nullptr, &mapped, &has_binding) != ERROR_SUCCESS)
    return DNS_ERROR_BAD_PACKET;
  DNS_STATUS status = DnsExtractRecordsFromMessage_W(message, length, records);
  if (status != ERROR_SUCCESS) return status;
  PDNS_RECORD* cursor = records;
  while (*cursor) {
    if (IsDnsFlatWireType((*cursor)->wType)) {
      PDNS_RECORD dropped = *cursor;
      *cursor = dropped->pNext;
      dropped->pNext = nullptr;
      TrueDnsFree(dropped, DnsFreeRecordList);
    } else cursor = &(*cursor)->pNext;
  }
  return *records ? ERROR_SUCCESS : DNS_INFO_NO_RECORDS;
}
std::vector<unsigned char> OpaqueCompatibilityPacket() {
  std::vector<unsigned char> packet(12, 0);
  WriteU16(packet.data() + 2, 0x8180);
  WriteU16(packet.data() + 4, 1);
  WriteU16(packet.data() + 6, 4);
  WriteU16(packet.data() + 8, 1);
  WriteU16(packet.data() + 10, 2);
  AddName(packet, "compat.fixture.test");
  packet.insert(packet.end(), {0, DNS_TYPE_HTTPS, 0, 1});
  const std::vector<unsigned char> opaque{0, 1, 0};
  AddRecord(packet, "compat.fixture.test", DNS_TYPE_NULL, opaque);
  std::vector<unsigned char> alias;
  AddName(alias, "alias.fixture.test");
  AddRecord(packet, "compat.fixture.test", DNS_TYPE_CNAME, alias);
  AddRecord(packet, "compat.fixture.test", DNS_TYPE_HTTPS, opaque);
  AddRecord(packet, "compat.fixture.test", DNS_TYPE_SVCB, opaque);
  AddRecord(packet, "authority.fixture.test", DNS_TYPE_NULL, opaque);
  AddRecord(packet, ".", DNS_TYPE_HTTPS, opaque);
  AddRecord(packet, ".", kDnsOpaqueCarrierType, opaque);
  return packet;
}
void CheckOpaqueCompatibilityRecords(PDNS_RECORD records, const char* phase) {
  // Windows puts the CNAME before the remaining answer records.
  const WORD types[] = {DNS_TYPE_CNAME, DNS_TYPE_NULL, DNS_TYPE_HTTPS,
                        DNS_TYPE_SVCB, DNS_TYPE_NULL, DNS_TYPE_HTTPS, kDnsOpaqueCarrierType};
  const unsigned char bytes[] = {0, 1, 0};
  for (unsigned i = 0; i < ARRAYSIZE(types); ++i) {
    if (!records || records->wType != types[i])
      std::fprintf(stderr, "Opaque phase=%s index=%u actual=%u expected=%u\n",
                   phase, i, records ? records->wType : 0, types[i]);
    Require(records && records->wType == types[i], "mixed NULL/SVCB/HTTPS type and order preserved");
    Require(records->Flags.S.Section == (i < 4 ? 1 : i == 4 ? 2 : 3) &&
            records->dwTtl == 10, "opaque RR section and TTL preserved");
    if (types[i] != DNS_TYPE_CNAME) {
      Require(records->wDataLength == sizeof(bytes) &&
              !memcmp(&records->Data, bytes, sizeof(bytes)),
              "opaque record bytes and length preserved");
    }
    records = records->pNext;
  }
  Require(records == nullptr, "no compatibility records invented");
}
void TestDownlevelOpaqueExtraction() {
  auto native_extractor = g_extract_dns_records;
  g_extract_dns_records = LegacyOpaqueExtractor;
  auto packet = OpaqueCompatibilityPacket();
  auto native_input = packet;
  DNS_BYTE_FLIP_HEADER_COUNTS(&reinterpret_cast<PDNS_MESSAGE_BUFFER>(native_input.data())->MessageHead);
  PDNS_RECORD records = nullptr;
  Require(LegacyOpaqueExtractor(reinterpret_cast<PDNS_MESSAGE_BUFFER>(native_input.data()),
          static_cast<WORD>(native_input.size()), &records) == ERROR_SUCCESS && records &&
          records->wType == DNS_TYPE_CNAME && !records->pNext,
          "downlevel extractor drops NULL/64/65/unknown but preserves parsed CNAME");
  TrueDnsFree(records, DnsFreeRecordList);
  records = nullptr;
  Require(ExtractProfileDnsRecords(packet.data(), static_cast<int>(packet.size()), &records) == ERROR_SUCCESS,
          "opaque compatibility succeeds against downlevel extractor");
  Require(packet == OpaqueCompatibilityPacket(), "record extraction preserves cached network-order wire bytes");
  CheckOpaqueCompatibilityRecords(records, "native");
  for (auto charset : {DnsCharSetUnicode, DnsCharSetAnsi, DnsCharSetUtf8}) {
    PDNS_RECORD copied = DnsRecordSetCopyEx(records, DnsCharSetUnicode, charset);
    Require(copied != nullptr, "native record charset copy retains compatibility allocation");
    CheckOpaqueCompatibilityRecords(copied, charset == DnsCharSetUnicode ? "copy-W" :
                                   charset == DnsCharSetAnsi ? "copy-A" : "copy-UTF8");
    TrueDnsFree(copied, DnsFreeRecordList);
  }
  TrueDnsFree(records, DnsFreeRecordList);
  records = nullptr;
  auto truncated = packet;
  truncated.pop_back();
  const int before = legacy_extract_calls;
  Require(ExtractProfileDnsRecords(truncated.data(), static_cast<int>(truncated.size()), &records) ==
          DNS_ERROR_BAD_PACKET && !records && legacy_extract_calls == before,
          "truncated opaque RDATA is rejected before native extraction");
  auto bad_owner = packet;
  unsigned count = 0;
  bool has_flat = false;
  DnsOpaqueWireRecord mappings[6] = {};
  Require(ScanDnsOpaqueWireRecords(packet.data(), static_cast<int>(packet.size()), mappings,
                                  &count, &has_flat) == ERROR_SUCCESS && count == 6,
          "mixed opaque scanner maps every future RR");
  bad_owner[mappings[0].owner_offset] = 0xff;
  bad_owner[mappings[0].owner_offset + 1] = 0xff;
  Require(ExtractProfileDnsRecords(bad_owner.data(), static_cast<int>(bad_owner.size()), &records) ==
          DNS_ERROR_BAD_PACKET && !records && legacy_extract_calls == before,
          "invalid compressed owner rejected before native extraction");
  g_extract_dns_records = native_extractor;
}

void QueryRouteAndFree(const char* name, unsigned type, DWORD options = 0,
                       DnsQueryFlavor flavor = kDnsFlavorW) {
  wchar_t wide_name[256] = {};
  Require(MultiByteToWideChar(CP_UTF8, 0, name, -1, wide_name, ARRAYSIZE(wide_name)) > 0,
          "cache fixture name conversion");
  PDNS_RECORD records = nullptr;
  Require(RouteDnsQuery(name, static_cast<WORD>(type), options, &records, nullptr,
          flavor, wide_name, "cache-fixture", nullptr, accepted_deadline) == ERROR_SUCCESS && records,
          "cached route returns owned records");
  Require(records->wType == type && records->dwTtl <= 10,
          "cached route preserves requested type and remaining TTL");
  TrueDnsFree(records, DnsFreeRecordList);
}
void ConfigureDirectDoh() {
  Configure(EnvBoxDnsUdp);
  profile.dns_upstreams[0].bootstrap_count = 1;
  strcpy_s(profile.dns_upstreams[0].bootstrap_ips[0], "1.1.1.1");
  accepted_deadline = GetTickCount64() + kDnsTotalBudgetMs;
}
void TestWaitingCacheCancellation(bool cancel_waiter) {
  ConfigureDirectDoh();
  transport_delay_ms = 50;
  std::thread owner([] { QueryRouteAndFree("waiting.cache.test", DNS_TYPE_HTTPS); });
  const ULONGLONG owner_deadline = GetTickCount64() + 1000;
  while (doh_calls.load() == 0 && GetTickCount64() < owner_deadline) Sleep(1);
  Require(doh_calls == 1, "in-flight owner reached transport");
  DnsTransportEndpoint endpoint;
  Require(DnsEndpoint(&profile, 0, &endpoint) == 1, "waiting cache endpoint");
  HANDLE cancelled = cancel_waiter ? CreateEventW(nullptr, TRUE, FALSE, nullptr) : nullptr;
  Require(!cancel_waiter || cancelled != nullptr, "waiting cancellation event");
  std::thread canceller;
  if (cancel_waiter) canceller = std::thread([cancelled] { Sleep(5); SetEvent(cancelled); });
  PDNS_RECORD records = nullptr;
  DNS_STATUS status = ERROR_TIMEOUT;
  const ULONGLONG started = GetTickCount64();
  const int result = DnsQueryOne(endpoint, "waiting.cache.test", DNS_TYPE_HTTPS,
      cancel_waiter ? accepted_deadline : started + 5, cancelled, nullptr, &records, &status);
  const ULONGLONG elapsed = GetTickCount64() - started;
  if (cancel_waiter) canceller.join();
  if (cancelled) CloseHandle(cancelled);
  owner.join();
  Require(result == (cancel_waiter ? -1 : 0) && !records && doh_calls == 1 && elapsed < 250,
          "waiting cache caller honors cancellation/deadline without another transport");
  std::printf("Cache waiting %s elapsed_ms=%llu DoH=%d\n",
              cancel_waiter ? "cancellation" : "deadline", elapsed, doh_calls.load());
}
void TestResponseCacheRoutes() {
  bool repeats_cached = true;
  for (unsigned type : {DNS_TYPE_A, DNS_TYPE_HTTPS, DNS_TYPE_SVCB}) {
    ConfigureDirectDoh();
    const ULONGLONG started = GetTickCount64();
    for (int i = 0; i < 100; ++i) QueryRouteAndFree("repeat.cache.test", type);
    std::printf("Cache route type=%u queries=100 DoH=%d seed=%d elapsed_ms=%llu\n",
                type, doh_calls.load(), seed_calls.load(), GetTickCount64() - started);
    std::fflush(stdout);
    repeats_cached = repeats_cached && doh_calls == 1 && seed_calls == 0 && host_calls == 0;
  }
  Require(repeats_cached, "100 repeated full-QTYPE routes use one transport request");
  double batch_ms[2] = {};
  int batch_requests[2] = {};
  for (int batch = 0; batch < 2; ++batch) {
    ConfigureDirectDoh();
    transport_delay_ms = 10;
    const auto started = std::chrono::steady_clock::now();
    for (int i = 0; i < 20; ++i)
      QueryRouteAndFree("latency.cache.test", DNS_TYPE_HTTPS,
                        batch == 0 ? DNS_QUERY_BYPASS_CACHE : 0);
    batch_ms[batch] = std::chrono::duration<double, std::milli>(
                        std::chrono::steady_clock::now() - started).count();
    batch_requests[batch] = doh_calls.load();
  }
  Require(batch_requests[0] == 20 && batch_requests[1] == 1,
          "simulated ten-millisecond transport benchmark request counts");
  std::printf("Simulated transport delay=10ms queries=20 bypass_requests=%d bypass_ms=%.2f cached_requests=%d cached_ms=%.2f\n",
              batch_requests[0], batch_ms[0], batch_requests[1], batch_ms[1]);
  TestWaitingCacheCancellation(true);
  TestWaitingCacheCancellation(false);
  ConfigureDirectDoh();
  transport_delay_ms = 50;
  HANDLE start = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  Require(start != nullptr, "parallel cache request gate");
  std::atomic<int> ready{0};
  std::vector<std::thread> callers;
  for (int i = 0; i < 8; ++i) callers.emplace_back([&] {
    ++ready;
    Require(WaitForSingleObject(start, 1000) == WAIT_OBJECT_0, "parallel cache request starts");
    QueryRouteAndFree("parallel.cache.test", DNS_TYPE_HTTPS);
  });
  const ULONGLONG gate_deadline = GetTickCount64() + 1000;
  while (ready.load() != 8 && GetTickCount64() < gate_deadline) Sleep(1);
  Require(ready.load() == 8, "all parallel callers reached gate");
  SetEvent(start);
  for (auto& caller : callers) caller.join();
  CloseHandle(start);
  std::printf("Cache concurrent queries=8 DoH=%d seed=%d\n", doh_calls.load(), seed_calls.load());
  Require(doh_calls == 1 && seed_calls == 0 && host_calls == 0,
          "eight concurrent first queries share one transport request");
  Configure(EnvBoxDnsUdp);
  accepted_deadline = GetTickCount64() + kDnsTotalBudgetMs;
  QueryRouteAndFree("first.cache.test", DNS_TYPE_HTTPS);
  Require(seed_calls == 2 && doh_calls == 1, "first hostname bootstrap follows CNAME using Profile seed");
  QueryRouteAndFree("second.cache.test", DNS_TYPE_HTTPS);
  Require(seed_calls == 2 && doh_calls == 2 && host_calls == 0,
          "distinct business query reuses bootstrap CNAME and address cache");

  ConfigureDirectDoh();
  for (auto flavor : {kDnsFlavorW, kDnsFlavorA, kDnsFlavorUtf8})
    QueryRouteAndFree("charset.cache.test", DNS_TYPE_A, 0, flavor);
  Require(doh_calls == 1, "native record charset copies share wire-response cache");
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_HTTPS);
  Require(doh_calls == 2, "QTYPE has an independent response cache entry");
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_A, DNS_QUERY_BYPASS_CACHE);
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_A, DNS_QUERY_BYPASS_CACHE);
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_A, DNS_QUERY_WIRE_ONLY);
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_A, DNS_QUERY_WIRE_ONLY);
  Require(doh_calls == 6, "BYPASS_CACHE and WIRE_ONLY each force a transport request");
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_A, DNS_QUERY_NO_RECURSION);
  Require(doh_calls == 7, "recursion option has a separate cache identity");
  profile.dns_upstreams[0].tls_revocation = 0;
  QueryRouteAndFree("charset.cache.test", DNS_TYPE_A);
  Require(doh_calls == 8, "TLS endpoint policy has a separate cache identity");

  ConfigureDirectDoh();
  QueryRouteAndFree("cancel.cache.test", DNS_TYPE_A);
  DnsTransportEndpoint endpoint;
  Require(DnsEndpoint(&profile, 0, &endpoint) == 1, "cached cancellation endpoint");
  HANDLE cancelled = CreateEventW(nullptr, TRUE, TRUE, nullptr);
  Require(cancelled != nullptr, "cached cancellation event");
  PDNS_RECORD records = nullptr;
  DNS_STATUS status = ERROR_TIMEOUT;
  Require(DnsQueryOne(endpoint, "cancel.cache.test", DNS_TYPE_A, accepted_deadline,
          cancelled, nullptr, &records, &status) == -1 && !records && doh_calls == 1,
          "signalled cancellation is honored before cache hit");
  CloseHandle(cancelled);
  Require(DnsQueryOne(endpoint, "cancel.cache.test", DNS_TYPE_A, GetTickCount64(),
          nullptr, nullptr, &records, &status) == 0 && !records && doh_calls == 1,
          "expired total deadline is honored before cache hit");
}
}

int main() {
  DnsWinsockScope winsock;
  Require(winsock.active != 0, "Winsock initialization");
  TrueDnsQuery_W = HostQuery; Truegetaddrinfo = HostAddress;
  TestResponseCacheRoutes();
  TestDownlevelOpaqueExtraction();
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
