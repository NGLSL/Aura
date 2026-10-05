#include <windows.h>
#include <stdio.h>
#include "envbox_dns_doh.h"

// DllMain deliberately makes no Rust calls. Query runs after LoadLibrary returns.
BOOL WINAPI DllMain(HINSTANCE, DWORD, LPVOID) { return TRUE; }

static int32_t ENVBOX_DOH_CALL Cancel(void* context) {
  ++*static_cast<unsigned*>(context);
  return 1;
}

extern "C" __declspec(dllexport) int __cdecl RunDohSmoke(unsigned* results, unsigned short port) {
  char url_text[128];
  int url_length = sprintf_s(url_text, "https://aura-doh-ffi.test:%u/dns-query", port);
  if (url_length <= 0 || port == 0) return 5;
  const uint8_t* url = reinterpret_cast<const uint8_t*>(url_text);
  const uint8_t ip[] = "127.0.0.1";
  const uint8_t packet[12] = {};
  uint8_t response[512] = {};
  uint32_t error = 0;
  int32_t length = envbox_doh_query(nullptr, 0, ip, sizeof(ip)-1, packet,
      sizeof(packet), response, sizeof(response), GetTickCount64()+1000,
      nullptr, nullptr, &error);
  results[0] = error;
  if (length != 0 || error != EnvBoxDohArgument) return 1;
  unsigned callbacks = 0;
  length = envbox_doh_query(url, url_length, ip, sizeof(ip)-1, packet,
      sizeof(packet), response, sizeof(response), GetTickCount64()+1000,
      Cancel, &callbacks, &error);
  results[1] = error; results[2] = callbacks;
  if (length != -1 || error != EnvBoxDohCancelledError || callbacks == 0) return 2;
  length = envbox_doh_query(url, url_length, ip, sizeof(ip)-1, packet,
      sizeof(packet), response, sizeof(response), GetTickCount64(),
      nullptr, nullptr, &error);
  results[3] = error;
  if (length != 0 || error != EnvBoxDohDeadline) return 3;
  // This exercises the native offline trust snapshot. Only an explicit
  // loopback endpoint is reachable; no trust store is modified or supplied.
  length = envbox_doh_query(url, url_length, ip, sizeof(ip)-1, packet,
      sizeof(packet), response, sizeof(response), GetTickCount64()+3000,
      nullptr, nullptr, &error);
  results[4] = error;
  if (length != 0 || (error != EnvBoxDohTrustSnapshot && error != EnvBoxDohNetwork &&
      error != EnvBoxDohCertificate && error != EnvBoxDohRevocationUnknown &&
      error != EnvBoxDohDisallowed && error != EnvBoxDohDeadline)) return 4;
  return 0;
}
