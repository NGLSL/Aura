#include "startup_gate.h"
#include "ipc_bootstrap.h"
#include "detours.h"
#include <windows.h>

namespace {
using EntryPoint = DWORD(WINAPI*)(void*);
EntryPoint g_entry = nullptr;

DWORD WINAPI GatedEntry(void* argument) {
  // This runs after the loader has released its lock. Rejecting a target here
  // prevents its EXE entry, not imported DLL initializers (a declared boundary).
  if (!EnvBoxIpcAwaitStartupRelease()) {
    ExitProcess(ERROR_ACCESS_DENIED);
  }
  return g_entry(argument);
}
}

int EnvBoxInstallStartupGate() {
  wchar_t enabled[8] = {};
  DWORD count = GetEnvironmentVariableW(L"ENVBOX_STARTUP_GATE", enabled, 8);
  if (count == 0) return 1;  // Legacy Compatibility, no timing claim.
  if (count != 1 || enabled[0] != L'1') return 0;
  auto image = reinterpret_cast<BYTE*>(GetModuleHandleW(nullptr));
  if (!image) return 0;
  auto dos = reinterpret_cast<IMAGE_DOS_HEADER*>(image);
  if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0) return 0;
  auto nt = reinterpret_cast<IMAGE_NT_HEADERS*>(image + dos->e_lfanew);
  if (nt->Signature != IMAGE_NT_SIGNATURE ||
      nt->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR_MAGIC ||
      (nt->OptionalHeader.Subsystem != IMAGE_SUBSYSTEM_WINDOWS_CUI &&
       nt->OptionalHeader.Subsystem != IMAGE_SUBSYSTEM_WINDOWS_GUI) ||
      nt->OptionalHeader.AddressOfEntryPoint == 0 ||
      nt->OptionalHeader.AddressOfEntryPoint >= nt->OptionalHeader.SizeOfImage) {
    return 0;
  }
  // EXE TLS callbacks precede entry and cannot be gated by an entry detour.
  // Only console/GUI EXEs without such callbacks are supported. Imported DLL
  // initializers remain outside this EXE-entry timing guarantee.
  auto tls = nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_TLS];
  if (tls.VirtualAddress != 0) {
    if (tls.Size < sizeof(IMAGE_TLS_DIRECTORY) ||
        tls.VirtualAddress > nt->OptionalHeader.SizeOfImage - sizeof(IMAGE_TLS_DIRECTORY))
      return 0;
    auto directory = reinterpret_cast<IMAGE_TLS_DIRECTORY*>(image + tls.VirtualAddress);
    // Conservatively reject even an empty callback array; do not follow an
    // arbitrary absolute pointer from a PE TLS directory in DllMain.
    if (directory->AddressOfCallBacks != 0) return 0;
  }
  g_entry = reinterpret_cast<EntryPoint>(image + nt->OptionalHeader.AddressOfEntryPoint);
  return DetourAttach(reinterpret_cast<PVOID*>(&g_entry), GatedEntry) == NO_ERROR;
}
