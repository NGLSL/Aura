#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <winhttp.h>
#include <wincrypt.h>
#include <cstdio>
#include <string>
#include <vector>

struct InternetHandle {
  HINTERNET value = nullptr;
  ~InternetHandle() { if (value) WinHttpCloseHandle(value); }
};

static void CALLBACK Observe(HINTERNET, DWORD_PTR, DWORD status, LPVOID info, DWORD length) {
  if (status == WINHTTP_CALLBACK_STATUS_RESOLVING_NAME || status == WINHTTP_CALLBACK_STATUS_NAME_RESOLVED || status == WINHTTP_CALLBACK_STATUS_CONNECTING_TO_SERVER || status == WINHTTP_CALLBACK_STATUS_CONNECTED_TO_SERVER) {
    std::printf("callback_status=%lu address_or_name=%ls\n", status, info ? static_cast<LPCWSTR>(info) : L"");
  }
  if (status == WINHTTP_CALLBACK_STATUS_SECURE_FAILURE && length == sizeof(DWORD)) {
    std::printf("certificate_failure_flags=0x%08lx\n", *static_cast<DWORD*>(info));
  }
}

static bool Option(HINTERNET handle, DWORD key, void* value, DWORD length) {
  BOOL ok = WinHttpSetOption(handle, key, value, length);
  std::printf("set_option=%lu ok=%d error=%lu\n", key, ok, ok ? 0 : GetLastError());
  return ok != FALSE;
}

int wmain(int argc, wchar_t** argv) {
  if (argc != 4) { std::fprintf(stderr, "usage: envbox-doh-prototype URL_HOST BOOTSTRAP_LITERAL PORT\n"); return 2; }
  in_addr ipv4{}; in6_addr ipv6{};
  if (InetPtonW(AF_INET, argv[2], &ipv4) != 1 && InetPtonW(AF_INET6, argv[2], &ipv6) != 1) { std::fprintf(stderr, "bootstrap must be a literal IP\n"); return 2; }
  wchar_t* end = nullptr; unsigned long port = wcstoul(argv[3], &end, 10);
  if (!end || *end || port == 0 || port > 65535) return 2;
  std::printf("bits=%u url_authority=%ls bootstrap=%ls port=%lu\n", unsigned(sizeof(void*) * 8), argv[1], argv[2], port);
  if (GetModuleHandleW(L"envbox-runtime64.dll") || GetModuleHandleW(L"envbox-runtime32.dll")) { std::fprintf(stderr, "injected prototype rejected\n"); return 3; }
  InternetHandle session{WinHttpOpen(L"Aura-bootstrap-prototype/1", WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0)};
  if (!session.value) return 3;
  WinHttpSetStatusCallback(session.value, Observe, WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS, 0);
  if (!WinHttpSetTimeouts(session.value, 2000, 4000, 4000, 4000)) return 3;
  DWORD tls = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
  DWORD protocols = WINHTTP_PROTOCOL_FLAG_HTTP2;
  BOOL yes = TRUE;
  if (!Option(session.value, WINHTTP_OPTION_SECURE_PROTOCOLS, &tls, sizeof(tls)) ||
      !Option(session.value, WINHTTP_OPTION_ENABLE_HTTP_PROTOCOL, &protocols, sizeof(protocols)) ||
      !Option(session.value, WINHTTP_OPTION_SERVER_CERT_CHAIN_BUILD_CACHE_ONLY, &yes, sizeof(yes))) {
    std::puts("gate=NO_GO_REQUIRED_SESSION_OPTION"); return 4;
  }
  InternetHandle connection{WinHttpConnect(session.value, argv[1], static_cast<INTERNET_PORT>(port), 0)};
  if (!connection.value) return 3;
  InternetHandle request{WinHttpOpenRequest(connection.value, L"POST", L"/dns-query", nullptr, WINHTTP_NO_REFERER, WINHTTP_DEFAULT_ACCEPT_TYPES, WINHTTP_FLAG_SECURE)};
  if (!request.value) return 3;
  DWORD disabled = WINHTTP_DISABLE_AUTHENTICATION | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_REDIRECTS;
  DWORD redirect = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
  DWORD autologon = WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH;
  std::wstring bootstrap(argv[2]);
  if (!Option(request.value, WINHTTP_OPTION_RESOLUTION_HOSTNAME, bootstrap.data(), DWORD((bootstrap.size() + 1) * sizeof(wchar_t))) ||
      !Option(request.value, WINHTTP_OPTION_DISABLE_FEATURE, &disabled, sizeof(disabled)) ||
      !Option(request.value, WINHTTP_OPTION_REDIRECT_POLICY, &redirect, sizeof(redirect)) ||
      !Option(request.value, WINHTTP_OPTION_AUTOLOGON_POLICY, &autologon, sizeof(autologon)) ||
      !Option(request.value, WINHTTP_OPTION_HTTP_PROTOCOL_REQUIRED, &protocols, sizeof(protocols))) {
    std::puts("gate=NO_GO_REQUIRED_REQUEST_OPTION"); return 4;
  }
  // RFC 8484 POST body: one A question for example.com; ID=0x1234.
  unsigned char query[] = {0x12,0x34,1,0,0,1,0,0,0,0,0,0,7,'e','x','a','m','p','l','e',3,'c','o','m',0,0,1,0,1};
  LPCWSTR headers = L"Content-Type: application/dns-message\r\nAccept: application/dns-message\r\n";
  std::wstring authority = std::wstring(L"Host: ") + argv[1];
  if (port != 443) authority += L":" + std::to_wstring(port);
  authority += L"\r\n";
  if (!WinHttpAddRequestHeaders(request.value, authority.c_str(), DWORD(-1), WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE)) return 3;
  if (!WinHttpSendRequest(request.value, headers, DWORD(-1), query, sizeof(query), sizeof(query), 0) || !WinHttpReceiveResponse(request.value, nullptr)) {
    std::printf("request_error=%lu\n", GetLastError()); return 5;
  }
  WINHTTP_CONNECTION_INFO info{}; info.cbSize = sizeof(info); DWORD size = sizeof(info);
  if (!WinHttpQueryOption(request.value, WINHTTP_OPTION_CONNECTION_INFO, &info, &size)) { std::printf("connection_info_error=%lu\n", GetLastError()); return 6; }
  wchar_t ip[64]{}; auto* address = reinterpret_cast<sockaddr*>(&info.RemoteAddress);
  if (address->sa_family == AF_INET) InetNtopW(AF_INET, &reinterpret_cast<sockaddr_in*>(address)->sin_addr, ip, 64);
  else if (address->sa_family == AF_INET6) InetNtopW(AF_INET6, &reinterpret_cast<sockaddr_in6*>(address)->sin6_addr, ip, 64);
  std::printf("remote_ip=%ls\n", ip);
  DWORD used = 0; size = sizeof(used); WinHttpQueryOption(request.value, WINHTTP_OPTION_HTTP_PROTOCOL_USED, &used, &size);
  std::printf("http_protocol_used=%lu\n", used);
  DWORD status = 0; size = sizeof(status); WinHttpQueryHeaders(request.value, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, nullptr, &status, &size, nullptr);
  std::printf("http_status=%lu\n", status);
  DWORD chain_flags = 0; size = sizeof(chain_flags);
  BOOL chain_observed = WinHttpQueryOption(request.value, WINHTTP_OPTION_SERVER_CERT_CHAIN_BUILD_FLAGS, &chain_flags, &size);
  std::printf("queried_chain_flags_ok=%d flags=0x%08lx error=%lu\n", chain_observed, chain_flags, chain_observed ? 0 : GetLastError());
  wchar_t type[256]{}; size = sizeof(type); WinHttpQueryHeaders(request.value, WINHTTP_QUERY_CONTENT_TYPE, nullptr, type, &size, nullptr);
  std::printf("content_type=%ls\n", type);
  wchar_t sent_headers[4096]{}; size = sizeof(sent_headers);
  if (WinHttpQueryHeaders(request.value, WINHTTP_QUERY_RAW_HEADERS_CRLF | WINHTTP_QUERY_FLAG_REQUEST_HEADERS, nullptr, sent_headers, &size, nullptr)) std::printf("generated_request_headers=%ls\n", sent_headers);
  DWORD available = 0; std::vector<unsigned char> body;
  for (;;) {
    if (!WinHttpQueryDataAvailable(request.value, &available)) { std::printf("body_query_error=%lu\n", GetLastError()); return 6; }
    if (!available) break;
    if (available > 65535 || body.size() + available > 65535) return 6;
    auto offset = body.size(); body.resize(offset + available); DWORD read = 0;
    if (!WinHttpReadData(request.value, body.data() + offset, available, &read)) return 6;
    body.resize(offset + read);
  }
  std::printf("dns_body_size=%zu\n", body.size());
  bool valid = status == 200 && used == WINHTTP_PROTOCOL_FLAG_HTTP2 && wcscmp(ip, argv[2]) == 0 && wcscmp(type, L"application/dns-message") == 0 && body.size() >= 12 && body[0] == 0x12 && body[1] == 0x34 && (body[2] & 0x80);
  std::printf("functional_result=%s\n", valid ? "PASS" : "FAIL");
  std::puts("zero_host_dns=UNPROVEN_CALLBACKS_ARE_NOT_PACKET_CAPTURE");
  return valid ? 0 : 6;
}
