#include <winsock2.h>
#include <windows.h>
#include <stdio.h>
int wmain(int argc, wchar_t** argv) {
  if (GetModuleHandleW(L"envbox-runtime64.dll") || GetModuleHandleW(L"envbox-runtime32.dll")) return 10;
  if (argc != 2) return 11;
  HMODULE module = LoadLibraryW(argv[1]);
  if (!module) { printf("load_error=%lu\n", GetLastError()); return 12; }
  using Run = int (__cdecl*)(unsigned*, unsigned short);
  auto run = reinterpret_cast<Run>(GetProcAddress(module, "RunDohSmoke"));
  if (!run) { printf("export_error=%lu\n", GetLastError()); FreeLibrary(module); return 13; }
  WSADATA ws;
  if (WSAStartup(MAKEWORD(2,2), &ws)) { FreeLibrary(module); return 14; }
  SOCKET reserved = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
  BOOL exclusive = TRUE;
  sockaddr_in endpoint = {};
  endpoint.sin_family = AF_INET;
  endpoint.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  int endpoint_length = sizeof(endpoint);
  if (reserved == INVALID_SOCKET || setsockopt(reserved, SOL_SOCKET,
      SO_EXCLUSIVEADDRUSE, reinterpret_cast<const char*>(&exclusive), sizeof(exclusive)) ||
      bind(reserved, reinterpret_cast<sockaddr*>(&endpoint), sizeof(endpoint)) ||
      getsockname(reserved, reinterpret_cast<sockaddr*>(&endpoint), &endpoint_length)) {
    printf("reserve_error=%d\n", WSAGetLastError());
    if (reserved != INVALID_SOCKET) closesocket(reserved);
    WSACleanup(); FreeLibrary(module); return 15;
  }
  // Keep our exclusive, bound, non-listening loopback port owned until return.
  // Another process cannot turn this negative smoke into a service exchange.
  unsigned short port = ntohs(endpoint.sin_port);
  unsigned results[5] = {};
  int status = run(results, port);
  printf("host_pid=%lu runtime_modules=0 pointer_bits=%zu loopback_port=%u status=%d argument=%u cancelled=%u callback_calls=%u deadline=%u offline_failure=%u\n",
      GetCurrentProcessId(), sizeof(void*)*8, port, status, results[0], results[1], results[2], results[3], results[4]);
  closesocket(reserved); WSACleanup();
  FreeLibrary(module);
  return status;
}
