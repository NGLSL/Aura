#include <winsock2.h>
#include <ws2tcpip.h>
#include <mswsock.h>
#include <windows.h>
#include <windns.h>
#include <winhttp.h>
#include <wincrypt.h>
#include <detours.h>
#include <atomic>
#include <cstdio>
#include <cstring>
#include "api-trap.h"

static constexpr int count = 24;
static std::atomic<DWORD> calls[count]{};
static bool installed = false;
static bool available[count]{};
static const char* names[count] = {"getaddrinfo", "GetAddrInfoW", "GetAddrInfoExA", "GetAddrInfoExW", "DnsQuery_A", "DnsQuery_W", "DnsQuery_UTF8", "DnsQueryEx", "DnsQueryRaw", "WinHttpOpen", "WinHttpGetProxyForUrl", "WinHttpGetProxyForUrlEx", "WinHttpGetProxyForUrlEx2", "WinHttpDetectAutoProxyConfigUrl", "WinHttpGetIEProxyConfigForCurrentUser", "WinHttpGetDefaultProxyConfiguration", "CertGetCertificateChain", "CertVerifyCertificateChainPolicy", "connect", "WSAConnect", "WSAIoctl", "sendto", "WSASendTo", "ConnectEx"};
static sockaddr_storage allowed{};
static bool endpoint_set = false;
static std::atomic<DWORD> connects_allowed{}, connects_denied{}, extension_denied{};
static std::atomic<LPFN_CONNECTEX> original_connectex4{}, original_connectex6{};
static PVOID originals[count]{};
static bool permit(SOCKET socket, const sockaddr* address, int size) {
 int type=0,type_size=sizeof(type);
 bool match=endpoint_set&&address&&getsockopt(socket,SOL_SOCKET,SO_TYPE,reinterpret_cast<char*>(&type),&type_size)==0&&type==SOCK_STREAM;
 if(match&&address->sa_family==AF_INET&&allowed.ss_family==AF_INET&&size>=sizeof(sockaddr_in)) {
  auto actual=reinterpret_cast<const sockaddr_in*>(address);auto expected=reinterpret_cast<const sockaddr_in*>(&allowed);
  match=actual->sin_port==expected->sin_port&&actual->sin_addr.S_un.S_addr==expected->sin_addr.S_un.S_addr;
 } else if(match&&address->sa_family==AF_INET6&&allowed.ss_family==AF_INET6&&size>=sizeof(sockaddr_in6)) {
  auto actual=reinterpret_cast<const sockaddr_in6*>(address);auto expected=reinterpret_cast<const sockaddr_in6*>(&allowed);
  match=actual->sin6_port==expected->sin6_port&&actual->sin6_scope_id==expected->sin6_scope_id&&std::memcmp(&actual->sin6_addr,&expected->sin6_addr,16)==0;
 } else match=false;
 if(match) connects_allowed.fetch_add(1);else {connects_denied.fetch_add(1);WSASetLastError(WSAEACCES);}
 return match;
}
static int WSAAPI h_connect(SOCKET socket,const sockaddr* address,int size) {
 calls[18].fetch_add(1);if(!permit(socket,address,size))return SOCKET_ERROR;
 return reinterpret_cast<decltype(&connect)>(originals[18])(socket,address,size);
}
static int WSAAPI h_WSAConnect(SOCKET socket,const sockaddr* address,int size,LPWSABUF caller,LPWSABUF callee,LPQOS sqos,LPQOS gqos) {
 calls[19].fetch_add(1);if(!permit(socket,address,size))return SOCKET_ERROR;
 return reinterpret_cast<decltype(&WSAConnect)>(originals[19])(socket,address,size,caller,callee,sqos,gqos);
}
static BOOL PASCAL h_ConnectEx(SOCKET socket,const sockaddr* address,int size,PVOID buffer,DWORD buffer_size,LPDWORD sent,LPOVERLAPPED overlapped) {
 calls[23].fetch_add(1);if(!permit(socket,address,size))return FALSE;
 auto original=address->sa_family==AF_INET?original_connectex4.load():original_connectex6.load();
 if(!original){WSASetLastError(WSAEOPNOTSUPP);return FALSE;}
 return original(socket,address,size,buffer,buffer_size,sent,overlapped);
}
static int WSAAPI h_WSAIoctl(SOCKET socket,DWORD code,LPVOID input,DWORD input_size,LPVOID output,DWORD output_size,LPDWORD returned,LPWSAOVERLAPPED overlapped,LPWSAOVERLAPPED_COMPLETION_ROUTINE completion) {
 const GUID connectex=WSAID_CONNECTEX;
 bool extension=code==SIO_GET_EXTENSION_FUNCTION_POINTER&&input&&input_size==sizeof(GUID)&&std::memcmp(input,&connectex,sizeof(GUID))==0;
 if(code==SIO_GET_EXTENSION_FUNCTION_POINTER
#ifdef SIO_GET_MULTIPLE_EXTENSION_FUNCTION_POINTER
    ||code==SIO_GET_MULTIPLE_EXTENSION_FUNCTION_POINTER
#endif
 ) {
  calls[20].fetch_add(1);
  // A returned extension pointer must be wrapped synchronously before use.
  // Other extensions (including datagram send/RIO) are unnecessary for this client.
  if(!extension||overlapped||completion){extension_denied.fetch_add(1);WSASetLastError(WSAEOPNOTSUPP);return SOCKET_ERROR;}
 }
 auto original=reinterpret_cast<decltype(&WSAIoctl)>(originals[20]);
 int result=original(socket,code,input,input_size,output,output_size,returned,overlapped,completion);
 if(extension&&result==0&&output&&output_size>=sizeof(LPFN_CONNECTEX)) {
  WSAPROTOCOL_INFOA protocol{};int protocol_size=sizeof(protocol);
  if(getsockopt(socket,SOL_SOCKET,SO_PROTOCOL_INFOA,reinterpret_cast<char*>(&protocol),&protocol_size)!=0)return SOCKET_ERROR;
  if(protocol.iAddressFamily!=AF_INET&&protocol.iAddressFamily!=AF_INET6){WSASetLastError(WSAEAFNOSUPPORT);return SOCKET_ERROR;}
  auto fn=*reinterpret_cast<LPFN_CONNECTEX*>(output);
  auto& slot=protocol.iAddressFamily==AF_INET?original_connectex4:original_connectex6;
  LPFN_CONNECTEX expected=nullptr;
  if(!slot.compare_exchange_strong(expected,fn)&&expected!=fn){WSASetLastError(WSAEOPNOTSUPP);return SOCKET_ERROR;}
  *reinterpret_cast<LPFN_CONNECTEX*>(output)=h_ConnectEx;
 }
 return result;
}
static int WSAAPI h_sendto(SOCKET,const char*,int,int,const sockaddr*,int) {calls[21].fetch_add(1);WSASetLastError(WSAEACCES);return SOCKET_ERROR;}
static int WSAAPI h_WSASendTo(SOCKET,LPWSABUF,DWORD,LPDWORD,DWORD,const sockaddr*,int,LPWSAOVERLAPPED,LPWSAOVERLAPPED_COMPLETION_ROUTINE) {calls[22].fetch_add(1);WSASetLastError(WSAEACCES);return SOCKET_ERROR;}
static int blocked(int index) { calls[index].fetch_add(1); SetLastError(ERROR_ACCESS_DENIED); return ERROR_ACCESS_DENIED; }
static int WSAAPI h_getaddrinfo(PCSTR,PCSTR,const ADDRINFOA*,PADDRINFOA* result) { blocked(0); if(result)*result=nullptr; WSASetLastError(WSAEACCES); return WSAEACCES; }
static int WSAAPI h_GetAddrInfoW(PCWSTR,PCWSTR,const ADDRINFOW*,PADDRINFOW* result) { blocked(1); if(result)*result=nullptr; WSASetLastError(WSAEACCES); return WSAEACCES; }
static int WSAAPI h_GetAddrInfoExA(PCSTR,PCSTR,DWORD,LPGUID,const ADDRINFOEXA*,PADDRINFOEXA* result,timeval*,LPOVERLAPPED,LPLOOKUPSERVICE_COMPLETION_ROUTINE,LPHANDLE) { blocked(2); if(result)*result=nullptr; WSASetLastError(WSAEACCES); return WSAEACCES; }
static int WSAAPI h_GetAddrInfoExW(PCWSTR,PCWSTR,DWORD,LPGUID,const ADDRINFOEXW*,PADDRINFOEXW* result,timeval*,LPOVERLAPPED,LPLOOKUPSERVICE_COMPLETION_ROUTINE,LPHANDLE) { blocked(3); if(result)*result=nullptr; WSASetLastError(WSAEACCES); return WSAEACCES; }
static DNS_STATUS WINAPI h_DnsQuery_A(PCSTR,WORD,DWORD,PVOID,PDNS_RECORD* result,PVOID*) {if(result)*result=nullptr;return blocked(4);}
static DNS_STATUS WINAPI h_DnsQuery_W(PCWSTR,WORD,DWORD,PVOID,PDNS_RECORD* result,PVOID*) {if(result)*result=nullptr;return blocked(5);}
static DNS_STATUS WINAPI h_DnsQuery_UTF8(PCSTR,WORD,DWORD,PVOID,PDNS_RECORD* result,PVOID*) {if(result)*result=nullptr;return blocked(6);}
static DNS_STATUS WINAPI h_DnsQueryEx(PDNS_QUERY_REQUEST,PDNS_QUERY_RESULT,PDNS_QUERY_CANCEL) {return blocked(7);}
static DNS_STATUS WINAPI h_DnsQueryRaw(void*,void*) {return blocked(8);}
static HINTERNET WINAPI h_WinHttpOpen(LPCWSTR,DWORD,LPCWSTR,LPCWSTR,DWORD) {blocked(9);return nullptr;}
static BOOL WINAPI h_WinHttpGetProxyForUrl(HINTERNET,LPCWSTR,WINHTTP_AUTOPROXY_OPTIONS*,WINHTTP_PROXY_INFO*) {blocked(10);return FALSE;}
static DWORD WINAPI h_WinHttpGetProxyForUrlEx(HINTERNET,PCWSTR,WINHTTP_AUTOPROXY_OPTIONS*,DWORD_PTR) {return blocked(11);}
static DWORD WINAPI h_WinHttpGetProxyForUrlEx2(HINTERNET,PCWSTR,WINHTTP_AUTOPROXY_OPTIONS*,DWORD,BYTE*,DWORD_PTR) {return blocked(12);}
static BOOL WINAPI h_WinHttpDetectAutoProxyConfigUrl(DWORD,LPWSTR*) {blocked(13);return FALSE;}
static BOOL WINAPI h_WinHttpGetIEProxyConfigForCurrentUser(WINHTTP_CURRENT_USER_IE_PROXY_CONFIG*) {blocked(14);return FALSE;}
static BOOL WINAPI h_WinHttpGetDefaultProxyConfiguration(WINHTTP_PROXY_INFO*) {blocked(15);return FALSE;}
static BOOL WINAPI h_CertGetCertificateChain(HCERTCHAINENGINE,PCCERT_CONTEXT,LPFILETIME,HCERTSTORE,PCERT_CHAIN_PARA,DWORD,LPVOID,PCCERT_CHAIN_CONTEXT* result) {blocked(16);if(result)*result=nullptr;return FALSE;}
static BOOL WINAPI h_CertVerifyCertificateChainPolicy(LPCSTR,PCCERT_CHAIN_CONTEXT,PCERT_CHAIN_POLICY_PARA,PCERT_CHAIN_POLICY_STATUS) {blocked(17);return FALSE;}

static PVOID hooks[count] = {reinterpret_cast<PVOID>(h_getaddrinfo),reinterpret_cast<PVOID>(h_GetAddrInfoW),reinterpret_cast<PVOID>(h_GetAddrInfoExA),reinterpret_cast<PVOID>(h_GetAddrInfoExW),reinterpret_cast<PVOID>(h_DnsQuery_A),reinterpret_cast<PVOID>(h_DnsQuery_W),reinterpret_cast<PVOID>(h_DnsQuery_UTF8),reinterpret_cast<PVOID>(h_DnsQueryEx),reinterpret_cast<PVOID>(h_DnsQueryRaw),reinterpret_cast<PVOID>(h_WinHttpOpen),reinterpret_cast<PVOID>(h_WinHttpGetProxyForUrl),reinterpret_cast<PVOID>(h_WinHttpGetProxyForUrlEx),reinterpret_cast<PVOID>(h_WinHttpGetProxyForUrlEx2),reinterpret_cast<PVOID>(h_WinHttpDetectAutoProxyConfigUrl),reinterpret_cast<PVOID>(h_WinHttpGetIEProxyConfigForCurrentUser),reinterpret_cast<PVOID>(h_WinHttpGetDefaultProxyConfiguration),reinterpret_cast<PVOID>(h_CertGetCertificateChain),reinterpret_cast<PVOID>(h_CertVerifyCertificateChainPolicy),reinterpret_cast<PVOID>(h_connect),reinterpret_cast<PVOID>(h_WSAConnect),reinterpret_cast<PVOID>(h_WSAIoctl),reinterpret_cast<PVOID>(h_sendto),reinterpret_cast<PVOID>(h_WSASendTo),nullptr};
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapAllowEndpoint(const char* ip,USHORT port) {
 if(installed)return ERROR_INVALID_STATE;if(!ip||!port)return ERROR_INVALID_PARAMETER;
 sockaddr_in v4{};v4.sin_family=AF_INET;v4.sin_port=htons(port);
 sockaddr_in6 v6{};v6.sin6_family=AF_INET6;v6.sin6_port=htons(port);
 if(InetPtonA(AF_INET,ip,&v4.sin_addr)==1)std::memcpy(&allowed,&v4,sizeof(v4));
 else if(InetPtonA(AF_INET6,ip,&v6.sin6_addr)==1)std::memcpy(&allowed,&v6,sizeof(v6));
 else return ERROR_INVALID_PARAMETER;
 endpoint_set=true;return ERROR_SUCCESS;
}
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapInstall() {
 if(installed)return ERROR_ALREADY_EXISTS;
 HMODULE modules[4]={LoadLibraryW(L"ws2_32.dll"),LoadLibraryW(L"dnsapi.dll"),LoadLibraryW(L"winhttp.dll"),LoadLibraryW(L"crypt32.dll")};
 for(auto module:modules)if(!module)return GetLastError();
 LONG status=DetourTransactionBegin();if(status)return status;
 status=DetourUpdateThread(GetCurrentThread());
 for(int i=0;!status&&i<count-1;++i) {
  auto module=modules[i<4||i>=18?0:i<9?1:i<16?2:3];
  originals[i]=reinterpret_cast<PVOID>(GetProcAddress(module,names[i]));
  available[i]=originals[i]!=nullptr;
  if(!available[i]&&i!=8&&i!=11&&i!=12){status=ERROR_PROC_NOT_FOUND;break;}
  if(available[i])status=DetourAttach(&originals[i],hooks[i]);
 }
 if(status){DetourTransactionAbort();return status;}
 status=DetourTransactionCommit();if(!status){installed=true;available[23]=true;}return status;
}
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapSnapshot(char* json,DWORD capacity) {
 if(!json||capacity<64)return ERROR_INSUFFICIENT_BUFFER;
 int used=std::snprintf(json,capacity,"{\"installed\":%s,\"apis\":[",installed?"true":"false");
 for(int i=0;i<count;++i) {
  if(used<0||static_cast<DWORD>(used)>=capacity)return ERROR_INSUFFICIENT_BUFFER;
  int added=std::snprintf(json+used,capacity-used,"%s{\"name\":\"%s\",\"available\":%s,\"calls\":%lu}",i?",":"",names[i],available[i]?"true":"false",static_cast<unsigned long>(calls[i].load()));
  if(added<0)return ERROR_INVALID_DATA;used+=added;
 }
 if(static_cast<DWORD>(used)+150>=capacity)return ERROR_INSUFFICIENT_BUFFER;
 std::snprintf(json+used,capacity-used,"],\"connects_allowed\":%lu,\"connects_denied\":%lu,\"extension_denied\":%lu}",static_cast<unsigned long>(connects_allowed.load()),static_cast<unsigned long>(connects_denied.load()),static_cast<unsigned long>(extension_denied.load()));return ERROR_SUCCESS;
}
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapSelfTest() {
 if(!installed)return ERROR_INVALID_STATE;
 ADDRINFOA* result=nullptr;
 int dns=getaddrinfo("trap.invalid",nullptr,nullptr,&result);
 HINTERNET http=WinHttpOpen(L"canary",WINHTTP_ACCESS_TYPE_NO_PROXY,nullptr,nullptr,0);
 BOOL chain=CertGetCertificateChain(nullptr,nullptr,nullptr,nullptr,nullptr,0,nullptr,nullptr);
 return dns==WSAEACCES&&!http&&!chain&&calls[0]&&calls[9]&&calls[16]?ERROR_SUCCESS:ERROR_INVALID_DATA;
}
