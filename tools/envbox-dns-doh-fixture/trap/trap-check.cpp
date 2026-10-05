#include <winsock2.h>
#include <ws2tcpip.h>
#include <mswsock.h>
#include <windows.h>
#include <cstdio>
#include <thread>
int wmain(int argc,wchar_t** argv) {
 if(argc!=2)return 2;
 WSADATA data{};if(WSAStartup(MAKEWORD(2,2),&data))return 3;
 SOCKET listener=socket(AF_INET,SOCK_STREAM,IPPROTO_TCP);
 sockaddr_in endpoint{};endpoint.sin_family=AF_INET;endpoint.sin_addr.S_un.S_addr=htonl(INADDR_LOOPBACK);
 if(bind(listener,reinterpret_cast<sockaddr*>(&endpoint),sizeof(endpoint))||listen(listener,1))return 4;
 int size=sizeof(endpoint);if(getsockname(listener,reinterpret_cast<sockaddr*>(&endpoint),&size))return 5;
 auto dll=LoadLibraryW(argv[1]);if(!dll)return 6;
 auto allow=reinterpret_cast<DWORD(WINAPI*)(const char*,USHORT)>(GetProcAddress(dll,"DoHApiTrapAllowEndpoint"));
 auto install=reinterpret_cast<DWORD(WINAPI*)()>(GetProcAddress(dll,"DoHApiTrapInstall"));
 auto selftest=reinterpret_cast<DWORD(WINAPI*)()>(GetProcAddress(dll,"DoHApiTrapSelfTest"));
 auto snapshot=reinterpret_cast<DWORD(WINAPI*)(char*,DWORD)>(GetProcAddress(dll,"DoHApiTrapSnapshot"));
 if(!allow||!install||!selftest||!snapshot)return 7;
 DWORD allow_status=allow("127.0.0.1",ntohs(endpoint.sin_port)),install_status=install();
 std::printf("allow=%lu install=%lu port=%u\n",allow_status,install_status,ntohs(endpoint.sin_port));
 if(allow_status||install_status)return 8;
 DWORD test=selftest();std::printf("selftest=%lu\n",test);if(test)return 9;
 SOCKET client=WSASocketW(AF_INET,SOCK_STREAM,IPPROTO_TCP,nullptr,0,WSA_FLAG_OVERLAPPED);
 sockaddr_in local{};local.sin_family=AF_INET;if(bind(client,reinterpret_cast<sockaddr*>(&local),sizeof(local)))return 10;
 GUID guid=WSAID_CONNECTEX;LPFN_CONNECTEX fn=nullptr;DWORD returned=0;
 if(WSAIoctl(client,SIO_GET_EXTENSION_FUNCTION_POINTER,&guid,sizeof(guid),&fn,sizeof(fn),&returned,nullptr,nullptr))return 11;
 OVERLAPPED overlapped{};overlapped.hEvent=CreateEventW(nullptr,TRUE,FALSE,nullptr);
 BOOL connected=fn(client,reinterpret_cast<sockaddr*>(&endpoint),sizeof(endpoint),nullptr,0,nullptr,&overlapped);
 if(!connected&&WSAGetLastError()!=ERROR_IO_PENDING)return 12;
 if(WaitForSingleObject(overlapped.hEvent,3000)!=WAIT_OBJECT_0)return 13;
 DWORD transferred=0,flags=0;if(!WSAGetOverlappedResult(client,&overlapped,&transferred,FALSE,&flags))return 14;
 if(setsockopt(client,SOL_SOCKET,SO_UPDATE_CONNECT_CONTEXT,nullptr,0))return 15;
 sockaddr_in peer{};size=sizeof(peer);if(getpeername(client,reinterpret_cast<sockaddr*>(&peer),&size))return 16;
 SOCKET accepted=accept(listener,nullptr,nullptr);if(accepted==INVALID_SOCKET)return 17;
 std::printf("connectex_peer=127.0.0.1:%u\n",ntohs(peer.sin_port));
 closesocket(accepted);closesocket(client);closesocket(listener);CloseHandle(overlapped.hEvent);
 sockaddr_in denied=endpoint;denied.sin_port=htons(ntohs(endpoint.sin_port)==65535?1:ntohs(endpoint.sin_port)+1);
 SOCKET rejected=socket(AF_INET,SOCK_STREAM,IPPROTO_TCP);
 if(connect(rejected,reinterpret_cast<sockaddr*>(&denied),sizeof(denied))!=SOCKET_ERROR||WSAGetLastError()!=WSAEACCES)return 18;
 if(WSAConnect(rejected,reinterpret_cast<sockaddr*>(&denied),sizeof(denied),nullptr,nullptr,nullptr,nullptr)!=SOCKET_ERROR||WSAGetLastError()!=WSAEACCES)return 19;
 closesocket(rejected);
 SOCKET udp=socket(AF_INET,SOCK_DGRAM,IPPROTO_UDP);
 if(sendto(udp,"x",1,0,reinterpret_cast<sockaddr*>(&endpoint),sizeof(endpoint))!=SOCKET_ERROR||WSAGetLastError()!=WSAEACCES)return 20;
 WSABUF buffer{1,const_cast<char*>("x")};DWORD sent=0;
 if(WSASendTo(udp,&buffer,1,&sent,0,reinterpret_cast<sockaddr*>(&endpoint),sizeof(endpoint),nullptr,nullptr)!=SOCKET_ERROR||WSAGetLastError()!=WSAEACCES)return 21;
 GUID sendmsg=WSAID_WSASENDMSG;PVOID forbidden_extension=nullptr;
 if(WSAIoctl(udp,SIO_GET_EXTENSION_FUNCTION_POINTER,&sendmsg,sizeof(sendmsg),&forbidden_extension,sizeof(forbidden_extension),&returned,nullptr,nullptr)!=SOCKET_ERROR||WSAGetLastError()!=WSAEOPNOTSUPP)return 23;
 closesocket(udp);
 char json[16384]{};if(snapshot(json,sizeof(json)))return 22;
 std::puts(json);
 // Hooks and loaded modules remain valid until process teardown.
 return 0;
}
