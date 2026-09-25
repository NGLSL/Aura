// IPC Bootstrap (ticket 40, Runtime side). Named Pipe client used when the
// process carries no ENVBOX_* Profile environment (packaged root, no
// Environment Block). Win32 env + profiles.toml remains the preferred path.
//
// Wire protocol (must match the Rust Host/Broker side):
//
//   Transport : Named Pipe, byte mode. Default name `\\.\pipe\envbox-runtime`.
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
//   Lists     : repeated keys (dns_server=, registry_path=), order preserved.
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
//   RUNTIME_READY   Client -> Host  pid=<u32>
//   HOOK_ERROR      Client -> Host  pid=<u32> api=<string> code=<u32>
//                                   [detail=<string>]
//   PROCESS_CREATED Client -> Host  pid=<u32> child_pid=<u32> [image=<string>]
//   PROCESS_EXITED  Client -> Host  pid=<u32> [exit_code=<u32>]
//
// PROFILE key mapping to RuntimeProfile: locale_name, ui_language, region,
// tz_windows, tz_iana, dns_mode, dns_server* -> dns_servers[],
// registry_path* -> registry_paths[], inherit_children, audit, instance_id,
// profile_id. locale_name + ui_language + region + tz_windows are required.
//
// Bootstrap flow: connect -> HELLO -> GET_PROFILE -> read until PROFILE ->
// close (V1). RUNTIME_READY is best-effort on a fresh short-lived connection
// after a successful Profile load (env path or IPC path). Every helper here
// is Fail Open: connect/timeout/protocol errors return failure or no-op and
// never crash or abort the process on their own.

#pragma once

#include "runtime_profile.h"

// Connect, HELLO + GET_PROFILE, receive one PROFILE into *out.
// Returns 1 on success. *out is zeroed first; incomplete PROFILE fails.
// Does not touch the process-wide g_profile (caller applies).
int EnvBoxIpcFetchProfile(RuntimeProfile* out);

// Best-effort RUNTIME_READY {pid}. No-op on any failure.
void EnvBoxIpcNotifyRuntimeReady(void);

// Best-effort HOOK_ERROR for hooks (Fail Open; never fatal).
void EnvBoxIpcNotifyHookError(const char* api_utf8, unsigned long code,
                              const char* detail_utf8);

// Best-effort lifecycle notices (PROCESS_CREATED / PROCESS_EXITED). Not wired
// into hooks in V1; provided so the Host side can implement the full set.
void EnvBoxIpcNotifyProcessCreated(unsigned long child_pid,
                                   const char* image_utf8);
void EnvBoxIpcNotifyProcessExited(unsigned long exit_code);
