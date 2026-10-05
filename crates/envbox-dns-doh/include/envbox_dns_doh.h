#pragma once
#include <stddef.h>
#include <stdint.h>
#if defined(_MSC_VER)
#define ENVBOX_DOH_CALL __cdecl
#else
#define ENVBOX_DOH_CALL
#endif
#ifdef __cplusplus
extern "C" {
#endif

// No pointer is retained after return. Input byte strings do not require NUL.
// Buffers must not overlap. URL<=2047, literal IP<=63; query/output12..65535.
// The URL determines the port. deadline is absolute GetTickCount64 milliseconds.
// Callback must not unwind; nonzero means cancelled. It runs on caller thread.
typedef int32_t (ENVBOX_DOH_CALL *EnvBoxDohCancelled)(void* context);
// >0 response bytes, 0 failure, -1 cancellation. error may be NULL.
int32_t ENVBOX_DOH_CALL envbox_doh_query(
  const uint8_t* url, size_t url_length,
  const uint8_t* literal_ip, size_t ip_length,
  const uint8_t* query, size_t query_length,
  uint8_t* response, size_t response_capacity,
  uint64_t deadline, EnvBoxDohCancelled cancelled, void* context,
  uint32_t* error);

enum EnvBoxDohError {
  EnvBoxDohNone=0, EnvBoxDohArgument=1, EnvBoxDohCancelledError=2,
  EnvBoxDohDeadline=3, EnvBoxDohNetwork=4, EnvBoxDohTls=5,
  EnvBoxDohCertificate=6, EnvBoxDohRevocationUnknown=7, EnvBoxDohRevoked=8,
  EnvBoxDohIdentity=9, EnvBoxDohDisallowed=10, EnvBoxDohTrustSnapshot=11,
  EnvBoxDohHttpStatus=12, EnvBoxDohMediaType=13, EnvBoxDohBodyLimit=14,
  EnvBoxDohHttp=15, EnvBoxDohPanic=16, EnvBoxDohContentEncoding=17
};
#ifdef __cplusplus
}
#endif
