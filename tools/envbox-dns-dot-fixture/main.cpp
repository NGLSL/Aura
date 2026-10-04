#include "../../runtime/src/dns_dot.h"
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <vector>
#include <thread>

static std::vector<unsigned char> ReadFile(const char* path) {
  std::ifstream file(path, std::ios::binary);
  return {std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>()};
}

int main(int argc, char** argv) {
  WSADATA data;
  if (WSAStartup(MAKEWORD(2, 2), &data)) return 2;
  unsigned char query[] = {0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
                          7, 'e', 'x', 'a', 'm', 'p', 'l', 'e', 3, 'c', 'o', 'm', 0, 0, 65, 0, 1};
  unsigned char response[65535];
  DnsDotError error;
  int failures = 0;
  if (argc == 9 || argc == 10) {
    // root DER, CRL DER (or '-'), IP, port, identity, budget ms, cancel delay ms
    auto root = ReadFile(argv[1]);
    HCERTSTORE store = CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, CERT_STORE_CREATE_NEW_FLAG, nullptr);
    if (!store || root.empty() || !CertAddEncodedCertificateToStore(store, X509_ASN_ENCODING,
        root.data(), static_cast<DWORD>(root.size()), CERT_STORE_ADD_ALWAYS, nullptr)) return 2;
    auto crl = ReadFile(argv[2]);
    if (!crl.empty() && !CertAddEncodedCRLToStore(store, X509_ASN_ENCODING, crl.data(),
        static_cast<DWORD>(crl.size()), CERT_STORE_ADD_ALWAYS, nullptr)) return 2;
    CERT_CHAIN_ENGINE_CONFIG config = {};
    config.cbSize = sizeof(config);
    config.hExclusiveRoot = store;
    config.cAdditionalStore = 1;
    config.rghAdditionalStore = &store;
    config.dwFlags = CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL | CERT_CHAIN_DISABLE_AIA |
                     CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE;
    HCERTCHAINENGINE engine = nullptr;
    if (!CertCreateCertificateChainEngine(&config, &engine)) return 2;
    HANDLE cancel = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    int delay = std::atoi(argv[8]);
    std::thread canceller;
    if (delay >= 0) canceller = std::thread([cancel, delay] { Sleep(delay); SetEvent(cancel); });
    ULONGLONG started = GetTickCount64();
    int repeats = argc == 10 ? std::atoi(argv[9]) : 1;
    if (repeats < 1 || repeats > 64) return 2;
    DWORD baseline = 0, final_count = 0;
    int result = 0;
    for (int i = 0; i < repeats; ++i) {
      result = DnsDotExchangeForTest(argv[3], static_cast<unsigned short>(std::atoi(argv[4])),
        argv[5], query, sizeof(query), response, sizeof(response),
        GetTickCount64() + std::atoi(argv[6]), cancel, engine, &error);
      if (i == 0) GetProcessHandleCount(GetCurrentProcess(), &baseline);
      if (result <= 0 && repeats > 1) break;
    }
    GetProcessHandleCount(GetCurrentProcess(), &final_count);
    // argv[7] is a case label recorded in the output, not a policy switch.
    std::printf("case=%s result=%d error=%d elapsed=%llu\n", argv[7], result,
      static_cast<int>(error), GetTickCount64() - started);
    if (repeats > 1) std::printf("handles baseline=%lu final=%lu repeats=%d\n", baseline, final_count, repeats);
    if (canceller.joinable()) canceller.join();
    CloseHandle(cancel);
    CertFreeCertificateChainEngine(engine);
    CertCloseStore(store, 0);
    WSACleanup();
    return result > 0 && final_count <= baseline ? 0 : 1;
  }
  if (argc == 4) {
    ULONGLONG started = GetTickCount64();
    int result = DnsDotExchange(argv[1], static_cast<unsigned short>(std::atoi(argv[2])),
      argv[3], query, sizeof(query), response, sizeof(response), started + 3000, nullptr, &error);
    std::printf("result=%d error=%d elapsed=%llu\n", result, static_cast<int>(error), GetTickCount64() - started);
    WSACleanup();
    return result > 0 ? 0 : 1;
  }
  HANDLE cancel = CreateEventW(nullptr, TRUE, TRUE, nullptr);
  int result = DnsDotExchange("127.0.0.1", 853, "fixture.test", query, sizeof(query),
    response, sizeof(response), GetTickCount64() + 1000, cancel, &error);
  if (result != -1 || error != DnsDotError::Cancelled) ++failures;
  CloseHandle(cancel);
  result = DnsDotExchange("127.0.0.1", 853, "fixture.test", query, sizeof(query),
    response, sizeof(response), GetTickCount64(), nullptr, &error);
  if (result || error != DnsDotError::Deadline) ++failures;
  result = DnsDotExchange("fixture.test", 853, "fixture.test", query, sizeof(query),
    response, sizeof(response), GetTickCount64() + 1000, nullptr, &error);
  if (result || error != DnsDotError::InvalidArgument) ++failures;
  std::printf("boundary failures=%d\n", failures);
  WSACleanup();
  return failures ? 1 : 0;
}
