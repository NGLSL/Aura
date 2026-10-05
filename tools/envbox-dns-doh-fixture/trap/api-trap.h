#pragma once
#include <windows.h>
// Load this DLL explicitly, then Install BEFORE constructing/running the backend.
// Keep it loaded until process exit. One fixture process, no host-wide hooks.
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapInstall();
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapSnapshot(char* json, DWORD capacity);
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapSelfTest();
extern "C" __declspec(dllexport) DWORD WINAPI DoHApiTrapAllowEndpoint(const char* literal_ip, USHORT port);
// SelfTest intentionally increments counters. Run in a SEPARATE fixture process.
