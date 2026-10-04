#include "dns_dot.h"
#include <ws2tcpip.h>
#define SECURITY_WIN32
#include <security.h>
#include <schannel.h>
#include <wincrypt.h>
#include <algorithm>
#include <cstring>
#include <new>
#include <vector>

namespace {
constexpr size_t kCipherLimit = 256 * 1024;
struct SocketOwner {
  SOCKET value = INVALID_SOCKET;
  ~SocketOwner() { if (value != INVALID_SOCKET) closesocket(value); }
};
struct TlsOwner {
  CredHandle credential = {};
  CtxtHandle context = {};
  bool has_credential = false, has_context = false;
  ~TlsOwner() {
    if (has_context) DeleteSecurityContext(&context);
    if (has_credential) FreeCredentialsHandle(&credential);
  }
};
struct CertOwner {
  PCCERT_CONTEXT value = nullptr;
  ~CertOwner() { if (value) CertFreeCertificateContext(value); }
};
struct ChainOwner {
  PCCERT_CHAIN_CONTEXT value = nullptr;
  ~ChainOwner() { if (value) CertFreeCertificateChain(value); }
};
struct TokenOwner {
  void* value = nullptr;
  ~TokenOwner() { if (value) FreeContextBuffer(value); }
};
struct Attempt {
  ULONGLONG deadline;
  HANDLE cancel;
  DnsDotError error = DnsDotError::None;
  bool Active() {
    if (cancel && WaitForSingleObject(cancel, 0) == WAIT_OBJECT_0) {
      error = DnsDotError::Cancelled; return false;
    }
    if (GetTickCount64() >= deadline) { error = DnsDotError::Deadline; return false; }
    return true;
  }
  bool Fail(DnsDotError cause) { error = cause; return false; }
};

bool Ready(SOCKET socket, bool write, Attempt& attempt) {
  while (attempt.Active()) {
    ULONGLONG remaining = attempt.deadline - GetTickCount64();
    // Active() and the clock read can straddle the deadline.
    if (remaining > 0x7fffffffULL) remaining = 0;
    if (!remaining) return attempt.Fail(DnsDotError::Deadline);
    timeval timeout = {0, static_cast<long>(std::min<ULONGLONG>(remaining, 50)) * 1000};
    fd_set set; FD_ZERO(&set); FD_SET(socket, &set);
    int result = select(0, write ? nullptr : &set, write ? &set : nullptr, nullptr, &timeout);
    if (result > 0) return attempt.Active();
    if (result == SOCKET_ERROR) return attempt.Fail(DnsDotError::Network);
  }
  return false;
}
bool Send(SOCKET socket, const unsigned char* data, size_t length, Attempt& attempt) {
  size_t offset = 0;
  while (offset < length) {
    if (!Ready(socket, true, attempt)) return false;
    int count = send(socket, reinterpret_cast<const char*>(data + offset),
                     static_cast<int>(length - offset), 0);
    if (count == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK) continue;
    if (count <= 0) return attempt.Fail(DnsDotError::Network);
    offset += count;
  }
  return true;
}
bool Receive(SOCKET socket, std::vector<unsigned char>& data, Attempt& attempt) {
  unsigned char block[16384];
  for (;;) {
    if (!Ready(socket, false, attempt)) return false;
    int count = recv(socket, reinterpret_cast<char*>(block), sizeof(block), 0);
    if (count == SOCKET_ERROR && WSAGetLastError() == WSAEWOULDBLOCK) continue;
    if (count <= 0) return attempt.Fail(DnsDotError::Network);
    if (data.size() + count > kCipherLimit) return attempt.Fail(DnsDotError::Tls);
    data.insert(data.end(), block, block + count);
    return true;
  }
}
bool Connect(const char* ip, unsigned short port, SocketOwner& socket, Attempt& attempt) {
  sockaddr_storage address = {};
  auto v4 = reinterpret_cast<sockaddr_in*>(&address);
  auto v6 = reinterpret_cast<sockaddr_in6*>(&address);
  int length = 0;
  if (InetPtonA(AF_INET, ip, &v4->sin_addr) == 1) {
    v4->sin_family = AF_INET; v4->sin_port = htons(port); length = sizeof(*v4);
  } else if (InetPtonA(AF_INET6, ip, &v6->sin6_addr) == 1) {
    v6->sin6_family = AF_INET6; v6->sin6_port = htons(port); length = sizeof(*v6);
  } else return attempt.Fail(DnsDotError::InvalidArgument);
  if (!attempt.Active()) return false;
  socket.value = ::socket(address.ss_family, SOCK_STREAM, IPPROTO_TCP);
  if (socket.value == INVALID_SOCKET) return attempt.Fail(DnsDotError::Network);
  u_long nonblocking = 1;
  if (ioctlsocket(socket.value, FIONBIO, &nonblocking)) return attempt.Fail(DnsDotError::Network);
  if (connect(socket.value, reinterpret_cast<sockaddr*>(&address), length)) {
    int error = WSAGetLastError();
    if (error != WSAEWOULDBLOCK && error != WSAEINPROGRESS) return attempt.Fail(DnsDotError::Network);
    if (!Ready(socket.value, true, attempt)) return false;
    length = sizeof(error);
    if (getsockopt(socket.value, SOL_SOCKET, SO_ERROR, reinterpret_cast<char*>(&error), &length) || error)
      return attempt.Fail(DnsDotError::Network);
  }
  return attempt.Active();
}
void KeepExtra(std::vector<unsigned char>& data, const SecBuffer& extra) {
  if (extra.BufferType == SECBUFFER_EXTRA && extra.cbBuffer <= data.size()) {
    size_t count = extra.cbBuffer;
    std::memmove(data.data(), data.data() + data.size() - count, count);
    data.resize(count);
  } else data.clear();
}
bool Handshake(SOCKET socket, const wchar_t* name, TlsOwner& tls,
               std::vector<unsigned char>& pending, Attempt& attempt) {
  // Explicit TLS 1.2 floor. TLS 1.3 can be enabled by replacing this credential
  // with SCH_CREDENTIALS after its SDK/OS compatibility is validated.
  SCHANNEL_CRED credentials = {};
  credentials.dwVersion = SCHANNEL_CRED_VERSION;
  credentials.grbitEnabledProtocols = SP_PROT_TLS1_2_CLIENT;
  credentials.dwFlags = SCH_CRED_MANUAL_CRED_VALIDATION | SCH_CRED_NO_DEFAULT_CREDS |
                        SCH_USE_STRONG_CRYPTO;
  TimeStamp expiry;
  SECURITY_STATUS status = AcquireCredentialsHandleW(nullptr,
    const_cast<wchar_t*>(UNISP_NAME_W), SECPKG_CRED_OUTBOUND, nullptr,
    &credentials, nullptr, nullptr, &tls.credential, &expiry);
  if (status != SEC_E_OK) return attempt.Fail(DnsDotError::Tls);
  tls.has_credential = true;
  bool first = true;
  constexpr ULONG flags = ISC_REQ_SEQUENCE_DETECT | ISC_REQ_REPLAY_DETECT |
    ISC_REQ_CONFIDENTIALITY | ISC_REQ_ALLOCATE_MEMORY | ISC_REQ_STREAM;
  while (attempt.Active()) {
    SecBuffer in[2] = {{static_cast<ULONG>(pending.size()), SECBUFFER_TOKEN, pending.data()},
                       {0, SECBUFFER_EMPTY, nullptr}};
    SecBufferDesc input = {SECBUFFER_VERSION, 2, in};
    SecBuffer out = {0, SECBUFFER_TOKEN, nullptr};
    SecBufferDesc output = {SECBUFFER_VERSION, 1, &out};
    ULONG attributes = 0;
    status = InitializeSecurityContextW(&tls.credential,
      first ? nullptr : &tls.context, const_cast<wchar_t*>(name), flags, 0,
      SECURITY_NATIVE_DREP, first ? nullptr : &input, 0,
      &tls.context, &output, &attributes, &expiry);
    // A context can be returned even on a failed continuation.
    if (SecIsValidHandle(&tls.context)) tls.has_context = true;
    TokenOwner token{out.pvBuffer};
    if (out.cbBuffer && out.pvBuffer && !Send(socket,
         static_cast<unsigned char*>(out.pvBuffer), out.cbBuffer, attempt)) return false;
    if (status == SEC_E_OK) {
      KeepExtra(pending, in[1]);
      if (!(attributes & ISC_RET_CONFIDENTIALITY) || !(attributes & ISC_RET_STREAM))
        return attempt.Fail(DnsDotError::Tls);
      SecPkgContext_ConnectionInfo connection = {};
      if (QueryContextAttributesW(&tls.context, SECPKG_ATTR_CONNECTION_INFO, &connection) != SEC_E_OK ||
          connection.dwProtocol != SP_PROT_TLS1_2_CLIENT) return attempt.Fail(DnsDotError::Tls);
      return attempt.Active();
    }
    if (status == SEC_E_INCOMPLETE_MESSAGE) {
      if (!Receive(socket, pending, attempt)) return false;
      continue;
    }
    if (status != SEC_I_CONTINUE_NEEDED) return attempt.Fail(DnsDotError::Tls);
    first = false;
    KeepExtra(pending, in[1]);
    if (pending.empty() && !Receive(socket, pending, attempt)) return false;
  }
  return false;
}

bool IpIdentity(PCCERT_CONTEXT certificate, const char* name, Attempt& attempt) {
  unsigned char binary[16];
  int length = 0;
  if (InetPtonA(AF_INET, name, binary) == 1) length = 4;
  else if (InetPtonA(AF_INET6, name, binary) == 1) length = 16;
  else return true; // The SSL policy handles DNS names below.
  PCERT_EXTENSION extension = CertFindExtension(szOID_SUBJECT_ALT_NAME2,
    certificate->pCertInfo->cExtension, certificate->pCertInfo->rgExtension);
  if (!extension) return attempt.Fail(DnsDotError::Identity);
  CERT_ALT_NAME_INFO* names = nullptr;
  DWORD size = 0;
  if (!CryptDecodeObjectEx(X509_ASN_ENCODING, X509_ALTERNATE_NAME,
       extension->Value.pbData, extension->Value.cbData, CRYPT_DECODE_ALLOC_FLAG,
       nullptr, &names, &size)) return attempt.Fail(DnsDotError::Certificate);
  bool match = false;
  for (DWORD i = 0; i < names->cAltEntry; ++i) {
    const auto& entry = names->rgAltEntry[i];
    if (entry.dwAltNameChoice == CERT_ALT_NAME_IP_ADDRESS &&
        entry.IPAddress.cbData == static_cast<DWORD>(length) &&
        std::memcmp(entry.IPAddress.pbData, binary, length) == 0) match = true;
  }
  LocalFree(names);
  return match || attempt.Fail(DnsDotError::Identity);
}
bool Verify(TlsOwner& tls, const char* name, const wchar_t* wide_name, Attempt& attempt,
            HCERTCHAINENGINE chain_engine) {
  if (!attempt.Active()) return false;
  CertOwner certificate;
  if (QueryContextAttributesW(&tls.context, SECPKG_ATTR_REMOTE_CERT_CONTEXT,
                              &certificate.value) != SEC_E_OK || !certificate.value)
    return attempt.Fail(DnsDotError::Certificate);
  CERT_CHAIN_PARA parameters = {};
  parameters.cbSize = sizeof(parameters);
  char* usages[] = {const_cast<char*>(szOID_PKIX_KP_SERVER_AUTH)};
  parameters.RequestedUsage.dwType = USAGE_MATCH_TYPE_AND;
  parameters.RequestedUsage.Usage.cUsageIdentifier = 1;
  parameters.RequestedUsage.Usage.rgpszUsageIdentifier = usages;
  ChainOwner chain;
  DWORD flags = CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL | CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY |
    CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT | CERT_CHAIN_DISABLE_AIA |
    CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE;
  if (!CertGetCertificateChain(chain_engine, certificate.value, nullptr,
       certificate.value->hCertStore, &parameters, flags, nullptr, &chain.value))
    return attempt.Fail(DnsDotError::Certificate);
  if (!attempt.Active()) return false;
  DWORD errors = chain.value->TrustStatus.dwErrorStatus;
  if (errors & CERT_TRUST_IS_REVOKED) return attempt.Fail(DnsDotError::Revoked);
  if (errors & ~(CERT_TRUST_REVOCATION_STATUS_UNKNOWN | CERT_TRUST_IS_OFFLINE_REVOCATION))
    return attempt.Fail(DnsDotError::Certificate);
  if (errors & (CERT_TRUST_REVOCATION_STATUS_UNKNOWN | CERT_TRUST_IS_OFFLINE_REVOCATION))
    return attempt.Fail(DnsDotError::RevocationUnavailable);
  if (errors) return attempt.Fail(DnsDotError::Certificate);
  SSL_EXTRA_CERT_CHAIN_POLICY_PARA ssl = {};
  ssl.cbSize = sizeof(ssl); ssl.dwAuthType = AUTHTYPE_SERVER;
  ssl.pwszServerName = const_cast<wchar_t*>(wide_name);
  CERT_CHAIN_POLICY_PARA policy = {};
  policy.cbSize = sizeof(policy); policy.pvExtraPolicyPara = &ssl;
  CERT_CHAIN_POLICY_STATUS status = {}; status.cbSize = sizeof(status);
  if (!CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, chain.value, &policy, &status))
    return attempt.Fail(DnsDotError::Certificate);
  if (status.dwError == CERT_E_CN_NO_MATCH) return attempt.Fail(DnsDotError::Identity);
  if (status.dwError) return attempt.Fail(DnsDotError::Certificate);
  return IpIdentity(certificate.value, name, attempt) && attempt.Active();
}
bool SendDns(SOCKET socket, TlsOwner& tls, const unsigned char* query,
             int length, Attempt& attempt) {
  SecPkgContext_StreamSizes sizes = {};
  if (QueryContextAttributesW(&tls.context, SECPKG_ATTR_STREAM_SIZES, &sizes) != SEC_E_OK ||
      !sizes.cbMaximumMessage || sizes.cbMaximumMessage > 65536 ||
      sizes.cbHeader + sizes.cbTrailer > 65536) return attempt.Fail(DnsDotError::Tls);
  std::vector<unsigned char> frame(length + 2);
  frame[0] = static_cast<unsigned char>(length >> 8);
  frame[1] = static_cast<unsigned char>(length);
  std::memcpy(frame.data() + 2, query, length);
  size_t offset = 0;
  while (offset < frame.size()) {
    if (!attempt.Active()) return false;
    ULONG count = static_cast<ULONG>(std::min<size_t>(frame.size() - offset, sizes.cbMaximumMessage));
    std::vector<unsigned char> record(sizes.cbHeader + count + sizes.cbTrailer);
    std::memcpy(record.data() + sizes.cbHeader, frame.data() + offset, count);
    SecBuffer buffers[4] = {{sizes.cbHeader, SECBUFFER_STREAM_HEADER, record.data()},
      {count, SECBUFFER_DATA, record.data() + sizes.cbHeader},
      {sizes.cbTrailer, SECBUFFER_STREAM_TRAILER, record.data() + sizes.cbHeader + count},
      {0, SECBUFFER_EMPTY, nullptr}};
    SecBufferDesc message = {SECBUFFER_VERSION, 4, buffers};
    if (EncryptMessage(&tls.context, 0, &message, 0) != SEC_E_OK) return attempt.Fail(DnsDotError::Tls);
    // Schannel can shrink the trailer; send each resulting buffer exactly.
    for (int i = 0; i < 3; ++i)
      if (!Send(socket, static_cast<unsigned char*>(buffers[i].pvBuffer), buffers[i].cbBuffer, attempt)) return false;
    offset += count;
  }
  return true;
}
int ReadDns(SOCKET socket, TlsOwner& tls, std::vector<unsigned char>& cipher,
            unsigned char* response, int capacity, Attempt& attempt) {
  std::vector<unsigned char> plain;
  int expected = -1;
  while (attempt.Active()) {
    if (cipher.empty() && !Receive(socket, cipher, attempt)) return 0;
    SecBuffer buffers[4] = {{static_cast<ULONG>(cipher.size()), SECBUFFER_DATA, cipher.data()},
      {0, SECBUFFER_EMPTY, nullptr}, {0, SECBUFFER_EMPTY, nullptr}, {0, SECBUFFER_EMPTY, nullptr}};
    SecBufferDesc message = {SECBUFFER_VERSION, 4, buffers};
    SECURITY_STATUS status = DecryptMessage(&tls.context, &message, 0, nullptr);
    if (status == SEC_E_INCOMPLETE_MESSAGE) {
      if (!Receive(socket, cipher, attempt)) return 0;
      continue;
    }
    // Renegotiation and close_notify before a full DNS frame fail this attempt.
    if (status != SEC_E_OK) { attempt.Fail(DnsDotError::Tls); return 0; }
    SecBuffer extra = {};
    for (const auto& buffer : buffers) {
      if (buffer.BufferType == SECBUFFER_DATA && buffer.cbBuffer) {
        if (plain.size() + buffer.cbBuffer > static_cast<size_t>(capacity) + 2) {
          attempt.Fail(DnsDotError::Packet); return 0;
        }
        auto data = static_cast<unsigned char*>(buffer.pvBuffer);
        plain.insert(plain.end(), data, data + buffer.cbBuffer);
      } else if (buffer.BufferType == SECBUFFER_EXTRA) extra = buffer;
    }
    KeepExtra(cipher, extra);
    if (expected < 0 && plain.size() >= 2) {
      expected = (static_cast<int>(plain[0]) << 8) | plain[1];
      if (expected < 12 || expected > capacity) { attempt.Fail(DnsDotError::Packet); return 0; }
    }
    if (expected >= 0 && plain.size() >= static_cast<size_t>(expected + 2)) {
      if (plain.size() != static_cast<size_t>(expected + 2)) { attempt.Fail(DnsDotError::Packet); return 0; }
      if (!attempt.Active()) return 0;
      std::memcpy(response, plain.data() + 2, expected);
      return expected;
    }
  }
  return 0;
}
}

static int Exchange(const char* literal_ip, unsigned short port, const char* server_name,
                   const unsigned char* query, int query_length, unsigned char* response,
                   int capacity, ULONGLONG deadline, HANDLE cancel_event, DnsDotError* error,
                   HCERTCHAINENGINE chain_engine) {
  Attempt attempt{deadline, cancel_event};
  int result = 0;
  try {
    if (!literal_ip || !port || !server_name || !*server_name || !query || !response ||
        query_length < 12 || query_length > 65535 || capacity < 12 || capacity > 65535) {
      attempt.Fail(DnsDotError::InvalidArgument);
    } else {
      wchar_t name[256];
      int length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, server_name, -1, name, 256);
      if (!length) attempt.Fail(DnsDotError::InvalidArgument);
      else {
        SocketOwner socket;
        TlsOwner tls;
        SecInvalidateHandle(&tls.credential); SecInvalidateHandle(&tls.context);
        std::vector<unsigned char> pending;
        if (Connect(literal_ip, port, socket, attempt) &&
            Handshake(socket.value, name, tls, pending, attempt) &&
            Verify(tls, server_name, name, attempt, chain_engine) &&
            SendDns(socket.value, tls, query, query_length, attempt))
          result = ReadDns(socket.value, tls, pending, response, capacity, attempt);
      }
    }
  } catch (const std::bad_alloc&) { attempt.Fail(DnsDotError::Memory); }
  if (attempt.error == DnsDotError::Cancelled) result = -1;
  if (error) *error = attempt.error;
  return result;
}

int DnsDotExchange(const char* literal_ip, unsigned short port, const char* server_name,
                   const unsigned char* query, int query_length, unsigned char* response,
                   int capacity, ULONGLONG deadline, HANDLE cancel_event, DnsDotError* error) {
  return Exchange(literal_ip, port, server_name, query, query_length, response,
                   capacity, deadline, cancel_event, error, nullptr);
}
#ifdef ENVBOX_DNS_TRANSPORT_TESTING
int DnsDotExchangeForTest(const char* literal_ip, unsigned short port, const char* server_name,
                          const unsigned char* query, int query_length, unsigned char* response,
                          int capacity, ULONGLONG deadline, HANDLE cancel_event,
                          HCERTCHAINENGINE chain_engine, DnsDotError* error) {
  // A null fixture engine cannot accidentally turn this into an alternate
  // verification policy; it retains exactly the production policy.
  return Exchange(literal_ip, port, server_name, query, query_length, response,
                   capacity, deadline, cancel_event, error, chain_engine);
}
#endif
