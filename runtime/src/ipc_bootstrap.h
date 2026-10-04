// IPC Bootstrap (V0.3, Runtime side). Named Pipe client - preferred config
// channel. ENVBOX_* structured value vars are the Win32 fallback; C++ never
// parses profiles.toml.
//
// Wire protocol (must match the Rust Host/Broker side):
//
//   Transport : Named Pipe, byte mode. Packaged roots first use
//               `\\.\pipe\envbox-runtime-pid-<pid>`. A standalone broker
//               requires an explicit ENVBOX_IPC_PIPE.
//               Override with ENVBOX_IPC_PIPE: either a bare pipe name
//               (prefixed automatically) or a full `\\.\pipe\...` path.
//   Framing   : one message per line, UTF-8, terminated by '\n' (0x0A).
//               A '\r' before '\n' is ignored. Max line 8 KiB.
//   Syntax    : MSG_NAME key=value [key=value ...]
//               value is a bare token (no space, no '"') or a double-quoted
//               string. Inside quotes: \\ -> \, \" -> ", \n -> LF, \r -> CR,
//               \t -> TAB. Unknown escapes keep the escaped character.
//               Integers are decimal; flags are 0|1. Senders quote any string
//               that may contain spaces or empty text.
//   Lists     : repeated keys (dns_server=, registry_path=, environment=), order preserved.
//               Receivers must ignore unknown keys and unknown message names.
//
// Message names (exact, case-sensitive):
//
//   HELLO           Client -> Host  pid=<u32> instance_id=<string>
//   GET_PROFILE     Client -> Host  pid=<u32> profile_id=<string>
//                                   (profile_id may be empty when the Host
//                                    resolves the Profile by pid)
//   PROFILE         Host -> Client  profile_id=<string> instance_id=<string>
//                                   locale_name=<string> ui_language=<string>
//                                   region=<string> tz_windows=<string>
//                                   tz_iana=<string> inherit_children=0|1
//                                   audit=0|1 dns_mode=0|1
//                                   [dns_server=<string>]*
//                                   [registry_path=<string>]*
//                                   [environment=<NAME=value>]*
//   RUNTIME_READY   Client -> Host  pid=<u32>
//                   Legacy notice only; never proves installed capabilities.
//   RUNTIME_IDENTITY Client -> Host pid=<u32> creation_time=<u64> protocol=1
//                   version=<product version> module_path=<actual DLL path>
//                   actual_profile=<encoded actual PROFILE> config_complete=0|1
//                   [hook=<group:attached_count>]* (up to 32 KiB).
//   STARTUP_GATE_READY Client -> Host pid= creation_time=
//   STARTUP_RELEASE Host -> Client pid= creation_time= (identity validated)
//   STARTUP_GATE_RELEASED Client -> Host pid= creation_time= (receipt ACK)
//   STARTUP_GATE_CONFIRMED Host -> Client pid= creation_time=
//                   All four run outside loader lock on one connection.
//                   This guards EXE entry, not imported DLL initializers.
//   HOOK_ERROR      Client -> Host  pid=<u32> api=<string> code=<u32>
//                                   [detail=<string>]
//   PROCESS_CREATED Client -> Host  pid=<u32> child_pid=<u32> [image=<string>]
//   PROCESS_EXITED  Client -> Host  pid=<u32> [exit_code=<u32>]
//   REGISTER_PROFILE Host -> Host   (same fields as PROFILE; Session Registry)
//   BIND_PID        Host -> Host    pid= profile_id= parent_pid=
//
// PROFILE key mapping to RuntimeProfile: locale_name, ui_language, region,
// tz_windows, tz_iana, dns_mode, dns_server* -> dns_servers[],
// registry_path* -> registry_paths[], environment* -> environment[],
// inherit_children, audit, instance_id, profile_id. Identity plus
// locale_name + ui_language + region + tz_windows must all be non-empty.
//
// Bootstrap flow: connect -> HELLO -> GET_PROFILE -> read until PROFILE ->
// close (V1). RUNTIME_READY is best-effort on a fresh short-lived connection
// after a successful Profile load (IPC path or ENVBOX_* fallback). Every
// helper here is Fail Open: connect/timeout/protocol errors return failure
// or no-op and never crash or abort the process on their own.

#pragma once

#include "runtime_profile.h"

// Connect, HELLO + GET_PROFILE, receive one PROFILE into *out.
// Returns 1 on success. *out is zeroed first; incomplete PROFILE fails.
// Does not touch the process-wide g_profile (caller applies).
int EnvBoxIpcFetchProfile(RuntimeProfile* out);

// Best-effort RUNTIME_READY {pid}. No-op on any failure.
void EnvBoxIpcNotifyRuntimeReady(void);  // Legacy, deliberately unverified.
// Outside loader lock only: the authenticated Host must approve the actual
// runtime identity before EXE entry. Failure is fatal to an enabled gate.
int EnvBoxIpcAwaitStartupRelease();
extern "C" DWORD WINAPI EnvBoxRuntimeReconnect(void* parameter);
// Query-only ownership escrow preserves a named Job across Host restarts.
int EnvBoxRecoveryJobOpen();
void EnvBoxRecoveryJobClose();
// Actual installed facts, emitted only after a successful hook commit.
int EnvBoxIpcNotifyRuntimeIdentity(HINSTANCE module, const int* counts,
                                    size_t count);

// Best-effort HOOK_ERROR for hooks (Fail Open; never fatal).
void EnvBoxIpcNotifyHookError(const char* api_utf8, unsigned long code,
                              const char* detail_utf8);

// Best-effort lifecycle notices (PROCESS_CREATED / PROCESS_EXITED). Child
// creation sends PROCESS_CREATED before ResumeThread so the Broker can bind
// the Profile before the injected Runtime asks for it during DllMain.
void EnvBoxIpcNotifyProcessCreated(unsigned long child_pid,
                                   const char* image_utf8);
// Controlled startup: a successful notice is not enough. Require an ACK
// binding the child's OS generation to the parent's immutable bundle.
int EnvBoxIpcRegisterChild(HANDLE child, unsigned long child_pid);
void EnvBoxIpcNotifyProcessExited(unsigned long exit_code);
void EnvBoxIpcNotifyProcessExitedPid(unsigned long pid,
                                    unsigned long exit_code);
