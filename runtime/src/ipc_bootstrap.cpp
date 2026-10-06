// Named Pipe client for IPC Bootstrap (ticket 40). Protocol: see ipc_bootstrap.h.
// No third-party libs: Win32 CreateFileW / ReadFile / WriteFile only.
// All failures are contained here (return 0 / no-op). Startup Fail Policy is
// enforced by the caller (EnvBoxLoadProfile), not by this module.

#include "ipc_bootstrap.h"
#include "service_bootstrap.h"

#include <stdio.h>
#include <string.h>

#include <string>
#include <memory>

namespace {

constexpr DWORD kConnectTimeoutMs = 2000;
constexpr DWORD kIoTimeoutMs = 3000;
constexpr size_t kMaxLine = 8192;
constexpr int kMaxKv = 256;

// Built from actual installed facts in DllMain; entry gate replays it on the
// same live connection as Ready. A fire-and-forget client can close before
// the Host authenticates it, so that optional notice cannot authorize entry.
std::string g_runtime_identity_wire;
HANDLE g_recovery_job = nullptr;
wchar_t g_recovery_job_name[256] = {};

// OVERLAPPED and its event must remain alive until cancellation completes.
void CancelAndDrain(HANDLE pipe, OVERLAPPED* operation) {
  CancelIoEx(pipe, operation);
  DWORD ignored = 0;
  GetOverlappedResult(pipe, operation, &ignored, TRUE);
}

// ---------------------------------------------------------------------------
// Pipe name resolution: ENVBOX_IPC_PIPE (bare name or \\.\pipe\... path).
// Packaged root default: a PID-scoped pipe; Win32/children use the explicit
// ENVBOX_IPC_PIPE inherited from their launcher or parent.
// ---------------------------------------------------------------------------
void GetPipePath(wchar_t* out, size_t cap) {
  DWORD n = GetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", out, (DWORD)cap);
  if (n == 0 || n >= cap) {
    _snwprintf_s(out, cap, _TRUNCATE, L"\\\\.\\pipe\\envbox-runtime-pid-%lu",
                 static_cast<unsigned long>(GetCurrentProcessId()));
    return;
  }
  // Bare name -> full pipe path.
  if (wcsncmp(out, L"\\\\.\\pipe\\", 9) != 0 &&
      wcsncmp(out, L"//./pipe/", 9) != 0) {
    wchar_t bare[256];
    wcsncpy_s(bare, out, _TRUNCATE);
    _snwprintf_s(out, cap, _TRUNCATE, L"\\\\.\\pipe\\%s", bare);
  }
}

// ---------------------------------------------------------------------------
// Overlapped read/write with deadline so a hung Host cannot stall DllMain.
// ---------------------------------------------------------------------------
DWORD RemainingMs(ULONGLONG deadline) {
  ULONGLONG now = GetTickCount64();
  return (now >= deadline) ? 0 : (DWORD)(deadline - now);
}

int WriteAll(HANDLE h, const char* data, DWORD len, ULONGLONG deadline) {
  DWORD sent = 0;
  while (sent < len) {
    DWORD timeout = RemainingMs(deadline);
    if (timeout == 0) {
      return 0;
    }
    OVERLAPPED ov = {};
    ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (ov.hEvent == nullptr) {
      return 0;
    }
    DWORD n = 0;
    BOOL ok = WriteFile(h, data + sent, len - sent, &n, &ov);
    if (!ok) {
      DWORD err = GetLastError();
      if (err != ERROR_IO_PENDING) {
        CloseHandle(ov.hEvent);
        return 0;
      }
      DWORD w = WaitForSingleObject(ov.hEvent, timeout);
      if (w != WAIT_OBJECT_0) {
        CancelAndDrain(h, &ov);
        CloseHandle(ov.hEvent);
        return 0;
      }
      if (!GetOverlappedResult(h, &ov, &n, FALSE)) {
        CloseHandle(ov.hEvent);
        return 0;
      }
    }
    CloseHandle(ov.hEvent);
    if (n == 0) return 0;
    sent += n;
  }
  return 1;
}

// Buffered line reader over a connected pipe handle.
struct PipeReader {
  HANDLE h;
  char buf[kMaxLine];
  size_t pos;  // consume cursor
  size_t end;  // valid data end
};

int ReadMore(PipeReader* r, ULONGLONG deadline) {
  if (r->pos > 0 && r->pos == r->end) {
    r->pos = 0;
    r->end = 0;
  } else if (r->pos > 0) {
    memmove(r->buf, r->buf + r->pos, r->end - r->pos);
    r->end -= r->pos;
    r->pos = 0;
  }
  if (r->end >= sizeof(r->buf)) {
    return 0;  // line overflow
  }
  DWORD timeout = RemainingMs(deadline);
  if (timeout == 0) {
    return 0;
  }
  OVERLAPPED ov = {};
  ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (ov.hEvent == nullptr) {
    return 0;
  }
  DWORD n = 0;
  BOOL ok = ReadFile(r->h, r->buf + r->end, (DWORD)(sizeof(r->buf) - r->end),
                     &n, &ov);
  if (!ok) {
    DWORD err = GetLastError();
    if (err != ERROR_IO_PENDING) {
      // Broken pipe / EOF counts as failure to read more.
      CloseHandle(ov.hEvent);
      return 0;
    }
    DWORD w = WaitForSingleObject(ov.hEvent, timeout);
    if (w != WAIT_OBJECT_0) {
      CancelAndDrain(r->h, &ov);
      CloseHandle(ov.hEvent);
      return 0;
    }
    if (!GetOverlappedResult(r->h, &ov, &n, FALSE)) {
      CloseHandle(ov.hEvent);
      return 0;
    }
  }
  CloseHandle(ov.hEvent);
  if (n == 0) {
    return 0;  // EOF
  }
  r->end += n;
  return 1;
}

// Read one '\n'-terminated line (without terminator). Strips trailing '\r'.
// Returns 1 on success, 0 on timeout / EOF / overflow.
int ReadLine(PipeReader* r, char* out, size_t cap, ULONGLONG deadline) {
  size_t o = 0;
  for (;;) {
    while (r->pos < r->end) {
      char c = r->buf[r->pos++];
      if (c == '\n') {
        if (o > 0 && out[o - 1] == '\r') {
          o--;
        }
        out[o] = '\0';
        return 1;
      }
      if (o + 1 >= cap) {
        return 0;  // line too long
      }
      out[o++] = c;
    }
    if (!ReadMore(r, deadline)) {
      return 0;
    }
  }
}

// ---------------------------------------------------------------------------
// Connect: retry until deadline (covers PostActivation early-start race).
// Opens with FILE_FLAG_OVERLAPPED so reads can time out.
// ---------------------------------------------------------------------------
HANDLE ConnectPipePath(const wchar_t* path, DWORD timeout_ms) {
  if (!EnvBoxServiceBootstrapConfigurationValid()) {
    SetLastError(ERROR_INVALID_DATA);
    return INVALID_HANDLE_VALUE;
  }
  ULONGLONG deadline = GetTickCount64() + timeout_ms;
  for (;;) {
    // Wait briefly for an instance; ignore failure and still try CreateFileW.
    // Keep the wait inside the caller's deadline so a missing broker cannot
    // add an extra fixed 100 ms after the retry window has expired.
    DWORD remaining = RemainingMs(deadline);
    if (remaining == 0) {
      break;
    }
    WaitNamedPipeW(path, remaining < 100 ? remaining : 100);
    HANDLE h = CreateFileW(path, kEnvBoxPipeClientAccess, 0, nullptr,
                           OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
    if (h != INVALID_HANDLE_VALUE) {
      if (EnvBoxServiceBootstrapRequired() && !EnvBoxValidateTrustedServicePipe(h)) {
        CloseHandle(h);
        SetLastError(ERROR_ACCESS_DENIED);
        return INVALID_HANDLE_VALUE;
      }
      DWORD mode = PIPE_READMODE_BYTE;
      SetNamedPipeHandleState(h, &mode, nullptr, nullptr);
      return h;
    }
    // Short connections are optional notices or Win32 descendants with a
    // complete ENVBOX_* fallback. Once their Broker has exited, waiting for
    // a nonexistent pipe would delay every child CreateProcess call.
    if (timeout_ms <= 100 && GetLastError() == ERROR_FILE_NOT_FOUND) {
      return INVALID_HANDLE_VALUE;
    }
    if (RemainingMs(deadline) == 0) {
      char msg[128];
      _snprintf_s(msg, sizeof(msg), _TRUNCATE,
                  "EnvBox IPC: connect failed (GetLastError=%lu)\n",
                  (unsigned long)GetLastError());
      OutputDebugStringA(msg);
      return INVALID_HANDLE_VALUE;
    }
    // The launcher publishes the first pipe instance before activation, so
    // this is normally never reached for a new root. Keep a short backoff for
    // PostActivation/late descendants without paying a 50 ms floor per retry.
    DWORD backoff = RemainingMs(deadline);
    if (backoff == 0) {
      break;
    }
    Sleep(backoff < 5 ? backoff : 5);
  }
  OutputDebugStringA("EnvBox IPC: connect deadline reached\n");
  return INVALID_HANDLE_VALUE;
}

HANDLE ConnectPipe(DWORD timeout_ms) {
  wchar_t path[256];
  GetPipePath(path, 256);
  return ConnectPipePath(path, timeout_ms);
}

void ClosePipe(HANDLE h) {
  if (h != INVALID_HANDLE_VALUE) {
    CloseHandle(h);
  }
}

// ---------------------------------------------------------------------------
// Message encode: MSG key=value ... with quoted string values.
// ---------------------------------------------------------------------------
void AppendQuoted(std::string* out, const char* val) {
  if (val == nullptr) {
    val = "";
  }
  out->push_back('"');
  for (const char* p = val; *p; ++p) {
    unsigned char c = (unsigned char)*p;
    if (c == '\\' || c == '"') {
      out->push_back('\\');
      out->push_back((char)c);
    } else if (c == '\n') {
      out->append("\\n");
    } else if (c == '\r') {
      out->append("\\r");
    } else if (c == '\t') {
      out->append("\\t");
    } else {
      out->push_back((char)c);
    }
  }
  out->push_back('"');
}

void AppendKv(std::string* out, const char* key, const char* val) {
  out->push_back(' ');
  out->append(key);
  out->push_back('=');
  AppendQuoted(out, val);
}

void AppendKvU32(std::string* out, const char* key, unsigned long v) {
  char num[32];
  _snprintf_s(num, sizeof(num), _TRUNCATE, "%lu", v);
  out->push_back(' ');
  out->append(key);
  out->push_back('=');
  out->append(num);
}

int SendLine(HANDLE h, const std::string& line, ULONGLONG deadline) {
  std::string wire = line;
  wire.push_back('\n');
  return WriteAll(h, wire.data(), (DWORD)wire.size(), deadline);
}

// ---------------------------------------------------------------------------
// Message decode: MSG key=value ... (bare or quoted values).
// ---------------------------------------------------------------------------
struct IpcMsg {
  char name[32];
  int count;
  char key[kMaxKv][32];
  char val[kMaxKv][2048];
};

int UnquoteInto(const char* src, size_t len, char* out, size_t cap) {
  size_t o = 0;
  size_t i = 0;
  while (i < len && o + 1 < cap) {
    char c = src[i++];
    if (c == '\\' && i < len) {
      char e = src[i++];
      if (e == 'n') {
        c = '\n';
      } else if (e == 'r') {
        c = '\r';
      } else if (e == 't') {
        c = '\t';
      } else {
        c = e;  // \\ \" and unknown escapes keep the char
      }
    }
    out[o++] = c;
  }
  out[o] = '\0';
  return i == len && cap > 0;
}

int ParseMsg(const char* line, IpcMsg* out) {
  memset(out, 0, sizeof(*out));
  size_t i = 0;
  size_t n = 0;
  // Message name (first token).
  while (line[i] && line[i] != ' ' && line[i] != '\t' && n + 1 < sizeof(out->name)) {
    out->name[n++] = line[i++];
  }
  out->name[n] = '\0';
  if (n == 0) {
    return 0;
  }
  while (line[i]) {
    while (line[i] == ' ' || line[i] == '\t') {
      i++;
    }
    if (!line[i]) {
      break;
    }
    if (out->count >= kMaxKv) {
      return 0;  // too many keys
    }
    // key
    n = 0;
    while (line[i] && line[i] != '=' && line[i] != ' ' && line[i] != '\t' &&
           n + 1 < sizeof(out->key[0])) {
      out->key[out->count][n++] = line[i++];
    }
    out->key[out->count][n] = '\0';
    if (line[i] != '=') {
      return 0;  // malformed: key without =
    }
    i++;  // skip '='
    // value: quoted or bare
    if (line[i] == '"') {
      i++;
      size_t start = i;
      while (line[i] && line[i] != '"') {
        if (line[i] == '\\' && line[i + 1]) {
          i++;
        }
        i++;
      }
      if (line[i] != '"') {
        return 0;  // unterminated quote
      }
      if (!UnquoteInto(line + start, i - start, out->val[out->count],
                  sizeof(out->val[0]))) return 0;
      i++;  // skip closing quote
    } else {
      size_t start = i;
      while (line[i] && line[i] != ' ' && line[i] != '\t') {
        i++;
      }
      size_t len = i - start;
      if (len >= sizeof(out->val[0])) {
        return 0;
      }
      memcpy(out->val[out->count], line + start, len);
      out->val[out->count][len] = '\0';
    }
    if (strcmp(out->key[out->count], "dns_server") != 0 &&
        strcmp(out->key[out->count], "environment") != 0 &&
        strcmp(out->key[out->count], "registry_path") != 0) {
      for (int previous = 0; previous < out->count; ++previous)
        if (strcmp(out->key[previous], out->key[out->count]) == 0) return 0;
    }
    out->count++;
  }
  return 1;
}

const char* MsgGet(const IpcMsg* m, const char* key) {
  for (int i = 0; i < m->count; i++) {
    if (strcmp(m->key[i], key) == 0) {
      return m->val[i];
    }
  }
  return nullptr;
}

// Collect repeated keys into a fixed array. Returns count.
int MsgGetAll(const IpcMsg* m, const char* key, char out[][64], int max_items,
              size_t item_cap) {
  int n = 0;
  for (int i = 0; i < m->count && n < max_items; i++) {
    if (strcmp(m->key[i], key) == 0) {
      strncpy_s(out[n], item_cap, m->val[i], _TRUNCATE);
      n++;
    }
  }
  return n;
}

template <size_t Width>
int MsgGetAllW(const IpcMsg* m, const char* key, wchar_t (*out)[Width],
               int max_items, size_t item_cap) {
  if (item_cap > Width) {
    item_cap = Width;
  }
  int n = 0;
  for (int i = 0; i < m->count && n < max_items; i++) {
    if (strcmp(m->key[i], key) == 0) {
      MultiByteToWideChar(CP_UTF8, 0, m->val[i], -1, out[n], (int)item_cap);
      out[n][item_cap - 1] = L'\0';
      n++;
    }
  }
  return n;
}

void Utf8ToWide(const char* src, wchar_t* dst, size_t dst_cap) {
  if (src == nullptr) {
    dst[0] = L'\0';
    return;
  }
  MultiByteToWideChar(CP_UTF8, 0, src, -1, dst, (int)dst_cap);
  dst[dst_cap - 1] = L'\0';
}

void WideToUtf8(const wchar_t* src, char* dst, size_t dst_cap) {
  if (src == nullptr) {
    dst[0] = '\0';
    return;
  }
  WideCharToMultiByte(CP_UTF8, 0, src, -1, dst, (int)dst_cap, nullptr, nullptr);
  dst[dst_cap - 1] = '\0';
}

int ParseFlag(const IpcMsg* m, const char* key, int def) {
  const char* v = MsgGet(m, key);
  if (v == nullptr) {
    return def;
  }
  return (v[0] == '1' && v[1] == '\0') ? 1 : 0;
}

int DnsMessageField(void* context, const char* key, char* out, size_t capacity) {
  const IpcMsg* message = static_cast<const IpcMsg*>(context);
  std::string joined;
  const char* value = nullptr;
  if (strcmp(key, "dns_servers") == 0) {
    for (int i = 0; i < message->count; ++i) if (strcmp(message->key[i], "dns_server") == 0) {
      if (!joined.empty()) joined += ';'; joined += message->val[i];
    }
    if (joined.empty()) return 0;
    value = joined.c_str();
  } else value = MsgGet(message, key);
  if (!value) return 0;
  if (strlen(value) >= capacity) return -1;
  strcpy_s(out, capacity, value);
  return 1;
}

int DecodeDnsMessage(const IpcMsg* message, RuntimeProfile* profile) {
  bool typed = MsgGet(message, "dns_config_version") != nullptr;
  for (int i = 0; i < message->count; ++i) {
    const char* key = message->key[i];
    if (typed && (strcmp(key, "dns_server") == 0 || strcmp(key, "dns_servers") == 0)) return 0;
    if (!typed && strncmp(key, "dns_", 4) == 0 && strcmp(key, "dns_mode") && strcmp(key, "dns_server")) return 0;
  }
  if (!MsgGet(message, "dns_mode") || !EnvBoxDecodeDnsConfiguration(profile, DnsMessageField, const_cast<IpcMsg*>(message))) return 0;
  if (typed) {
    std::string keys;
    auto collect = [](void* context, const char* key, const char*) -> int { auto* names = static_cast<std::string*>(context); *names += '|'; *names += key; *names += '|'; return 1; };
    if (!EnvBoxEmitDnsConfiguration(profile, collect, &keys)) return 0;
    for (int i = 0; i < message->count; ++i) if (strncmp(message->key[i], "dns_", 4) == 0) {
      std::string key = "|"; key += message->key[i]; key += '|';
      if (keys.find(key) == std::string::npos) return 0;
    }
  }
  return 1;
}

// Apply a decoded PROFILE message into *out. Returns 1 when required fields
// are present (same completeness rule as the ENVBOX_* value fallback).
int FillProfileFromMsg(const IpcMsg* m, RuntimeProfile* out) {
  const char* v;
  if ((v = MsgGet(m, "profile_id")) != nullptr) {
    Utf8ToWide(v, out->profile_id, 64);
  }
  if ((v = MsgGet(m, "instance_id")) != nullptr) {
    Utf8ToWide(v, out->instance_id, 64);
  }
  if ((v = MsgGet(m, "locale_name")) != nullptr) {
    Utf8ToWide(v, out->locale_name, 85);
    out->has_locale = 1;
  }
  if ((v = MsgGet(m, "ui_language")) != nullptr) {
    Utf8ToWide(v, out->ui_language, 85);
    out->has_ui = 1;
  }
  if ((v = MsgGet(m, "region")) != nullptr) {
    Utf8ToWide(v, out->region, 16);
    out->has_region = 1;
  }
  if ((v = MsgGet(m, "tz_windows")) != nullptr) {
    Utf8ToWide(v, out->tz_windows, 128);
    out->has_tz = 1;
  }
  if ((v = MsgGet(m, "tz_iana")) != nullptr) {
    Utf8ToWide(v, out->tz_iana, 128);
  }
  out->inherit_children = ParseFlag(m, "inherit_children", 1);
  out->audit = ParseFlag(m, "audit", 0);
  const char* identity_keys[] = {"identity_computer_name", "identity_user_name", "identity_mac_address", "identity_machine_guid"};
  for (int i = 0; i < m->count; ++i) if (strncmp(m->key[i], "identity_", 9) == 0) {
    bool known = false;
    for (const char* key : identity_keys) if (strcmp(key, m->key[i]) == 0) known = true;
    if (!known) return 0;
    for (int j = 0; j < i; ++j) if (strcmp(m->key[j], m->key[i]) == 0) return 0;
  }
  if (!DecodeDnsMessage(m, out) || !EnvBoxDecodeIdentityConfiguration(out, DnsMessageField, const_cast<IpcMsg*>(m))) return 0;
  // VirtualView + empty dns_servers stays VirtualView: no virtual resolve, but
  // Network Guard can close external UDP/53 (empty allowlist).
  out->registry_path_count =
      MsgGetAllW(m, "registry_path", out->registry_paths, ENVBOX_REG_MAX, 128);
  out->environment_count = MsgGetAllW(m, "environment", out->environment,
                                      ENVBOX_ENV_MAX, ENVBOX_ENV_ENTRY_MAX);

  // Browser / Network Guard WebRTC policy token (ticket 51). C++ stores only.
  if ((v = MsgGet(m, "webrtc")) != nullptr) {
    Utf8ToWide(v, out->webrtc_policy, 32);
  }

  return out->profile_id[0] != L'\0' && out->instance_id[0] != L'\0' &&
         out->has_locale && out->locale_name[0] != L'\0' && out->has_ui &&
         out->ui_language[0] != L'\0' && out->has_region &&
         out->region[0] != L'\0' && out->has_tz &&
         out->tz_windows[0] != L'\0';
}

// Send one fire-and-forget message on a fresh connection. Best-effort.
void Notify(const char* name, const std::string& body, DWORD connect_ms = 100) {
  // Lifecycle notices are best-effort. After Aura exits, child creation must
  // not stall for the full bootstrap timeout before using inherited values.
  HANDLE h = ConnectPipe(connect_ms);
  if (h == INVALID_HANDLE_VALUE) {
    return;
  }
  ULONGLONG deadline = GetTickCount64() + kIoTimeoutMs;
  std::string line = name;
  line += body;
  SendLine(h, line, deadline);
  ClosePipe(h);
}

}  // namespace

int EnvBoxIpcFetchProfile(RuntimeProfile* out) {
  if (out == nullptr) {
    return 0;
  }
  memset(out, 0, sizeof(*out));
  out->inherit_children = 1;

  wchar_t profile_hint[64] = {};
  GetEnvironmentVariableW(L"ENVBOX_PROFILE_ID", profile_hint, 64);
  // Packaged roots need a retry window because they have no Environment
  // Block. Descendants already carry a complete ENVBOX_* fallback and only
  // make a short broker attempt so they remain responsive after Aura exits.
  wchar_t pipe_path[256];
  GetPipePath(pipe_path, 256);
  HANDLE h = ConnectPipePath(pipe_path,
                             profile_hint[0] != L'\0' ? 100 : kConnectTimeoutMs);
  if (h == INVALID_HANDLE_VALUE) {
    return 0;
  }

  ULONGLONG deadline = GetTickCount64() + kIoTimeoutMs;
  unsigned long pid = GetCurrentProcessId();

  wchar_t instance_w[64] = {};
  GetEnvironmentVariableW(L"ENVBOX_INSTANCE_ID", instance_w, 64);
  char instance_utf8[64];
  WideToUtf8(instance_w, instance_utf8, sizeof(instance_utf8));

  wchar_t profile_w[64] = {};
  GetEnvironmentVariableW(L"ENVBOX_PROFILE_ID", profile_w, 64);
  char profile_utf8[64];
  WideToUtf8(profile_w, profile_utf8, sizeof(profile_utf8));

  // HELLO pid=... instance_id=...
  {
    std::string line = "HELLO";
    AppendKvU32(&line, "pid", pid);
    AppendKv(&line, "instance_id", instance_utf8);
    if (!SendLine(h, line, deadline)) {
      OutputDebugStringA("EnvBox IPC: HELLO send failed\n");
      ClosePipe(h);
      return 0;
    }
  }

  // GET_PROFILE pid=... profile_id=...
  {
    std::string line = "GET_PROFILE";
    AppendKvU32(&line, "pid", pid);
    AppendKv(&line, "profile_id", profile_utf8);
    if (!SendLine(h, line, deadline)) {
      OutputDebugStringA("EnvBox IPC: GET_PROFILE send failed\n");
      ClosePipe(h);
      return 0;
    }
  }

  // Read lines until PROFILE (ignore unknown/other names).
  PipeReader reader = {};
  reader.h = h;
  char line[kMaxLine];
  auto msg = std::make_unique<IpcMsg>();
  int got = 0;
  for (;;) {
    if (!ReadLine(&reader, line, sizeof(line), deadline)) {
      OutputDebugStringA("EnvBox IPC: PROFILE read failed (timeout/eof)\n");
      ClosePipe(h);
      return 0;
    }
    if (!ParseMsg(line, msg.get())) {
      OutputDebugStringA("EnvBox IPC: malformed message ignored\n");
      continue;
    }
    if (strcmp(msg->name, "ERROR") == 0) {
      OutputDebugStringA("EnvBox IPC: bootstrap denied by host\n");
      ClosePipe(h);
      return 0;
    }
    if (strcmp(msg->name, "PROFILE") == 0) {
      if (!FillProfileFromMsg(msg.get(), out)) {
        OutputDebugStringA("EnvBox IPC: PROFILE incomplete\n");
        ClosePipe(h);
        return 0;
      }
      SetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", pipe_path);
      got = 1;
      break;
    }
    // HELLO ack or other traffic: keep reading.
  }

  ClosePipe(h);
  return got ? 1 : 0;
}

void EnvBoxIpcNotifyRuntimeReady(void) {
  std::string body;
  AppendKvU32(&body, "pid", (unsigned long)GetCurrentProcessId());
  Notify("RUNTIME_READY", body);
}

int EnvBoxRecoveryJobOpen() {
  SetLastError(ERROR_SUCCESS);
  DWORD length = GetEnvironmentVariableW(L"ENVBOX_RECOVERY_JOB_NAME", g_recovery_job_name, 256);
  if (length == 0) {
    if (GetLastError() == ERROR_ENVVAR_NOT_FOUND) return 1;
    SetLastError(ERROR_INVALID_DATA); return 0;
  }
  if (length >= 256) { SetLastError(ERROR_INVALID_DATA); return 0; }
  HANDLE job = OpenJobObjectW(JOB_OBJECT_QUERY, FALSE, g_recovery_job_name);
  if (job == nullptr) return 0;
  BOOL member = FALSE;
  JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = {};
  BOOL valid = IsProcessInJob(GetCurrentProcess(), job, &member) && member &&
      QueryInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof(limits), nullptr) &&
      !(limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
  if (!valid) {
    DWORD failure = GetLastError(); CloseHandle(job); g_recovery_job_name[0] = L'\0';
    SetLastError(failure == ERROR_SUCCESS ? ERROR_ACCESS_DENIED : failure); return 0;
  }
  g_recovery_job = job;
  return 1;
}

void EnvBoxRecoveryJobClose() {
  if (g_recovery_job != nullptr) { CloseHandle(g_recovery_job); g_recovery_job = nullptr; }
  g_recovery_job_name[0] = L'\0';
}

int EnvBoxIpcAwaitStartupRelease() {
  // A single deadline covers connection, release, acknowledgement and final
  // confirmation; an idle/disconnected Host never releases application entry.
  ULONGLONG deadline = GetTickCount64() + 5000;
  HANDLE pipe = ConnectPipe(RemainingMs(deadline));
  if (pipe == INVALID_HANDLE_VALUE) return 0;
  FILETIME created = {}, exited = {}, kernel = {}, user = {};
  int success = 0;
  if (!g_runtime_identity_wire.empty() &&
      GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user) &&
      SendLine(pipe, g_runtime_identity_wire, deadline)) {
    ULONGLONG generation = (static_cast<ULONGLONG>(created.dwHighDateTime) << 32)
                           | created.dwLowDateTime;
    char identity[128] = {};
    _snprintf_s(identity, sizeof(identity), _TRUNCATE,
        " pid=%lu creation_time=%llu", GetCurrentProcessId(), generation);
    PipeReader reader = {};
    reader.h = pipe;
    char line[256] = {};
    const char* requests[] = {"STARTUP_GATE_READY", "STARTUP_GATE_RELEASED"};
    const char* replies[] = {"STARTUP_RELEASE", "STARTUP_GATE_CONFIRMED"};
    success = 1;
    for (int step = 0; step < 2; ++step) {
      std::string expected = replies[step]; expected += identity;
      if (!SendLine(pipe, std::string(requests[step]) + identity, deadline)
          || !ReadLine(&reader, line, sizeof(line), deadline)
          || expected != line) { success = 0; break; }
    }
  }
  CloseHandle(pipe);
  return success;
}

// Dedicated remote-thread entry; never called during loader initialization.
// No borrowed remote parameters, no new DLL load, and no profile mutation.
extern "C" const char EnvBoxRuntimeCapabilities[256] =
    "protocol=1;profile_dns_schema=1;entry_gate=1;reconnect=1;dns_udp=1;dns_tcp=1;dns_dot=1;dns_doh=1";

extern "C" DWORD WINAPI EnvBoxRuntimeReconnect(void* parameter) {
  if (parameter != nullptr || g_runtime_identity_wire.empty()) return ERROR_INVALID_PARAMETER;
  const ULONGLONG deadline = GetTickCount64() + 3000;
  HANDLE pipe = ConnectPipe(RemainingMs(deadline));
  if (pipe == INVALID_HANDLE_VALUE) return ERROR_TIMEOUT;
  FILETIME created = {}, exited = {}, kernel = {}, user = {};
  DWORD result = ERROR_ACCESS_DENIED;
  if (GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user)) {
    std::string request = "RUNTIME_RECONNECT";
    request += g_runtime_identity_wire.substr(strlen("RUNTIME_IDENTITY"));
    char expected[160];
    _snprintf_s(expected, sizeof(expected), _TRUNCATE,
        "RUNTIME_RECONNECTED pid=%lu creation_time=%llu", GetCurrentProcessId(),
        (static_cast<ULONGLONG>(created.dwHighDateTime) << 32) | created.dwLowDateTime);
    PipeReader reader = {}; reader.h = pipe;
    char response[256] = {};
    char challenge_prefix[160];
    _snprintf_s(challenge_prefix, sizeof(challenge_prefix), _TRUNCATE,
        "RUNTIME_RECONNECT_CHALLENGE pid=%lu creation_time=%llu nonce=", GetCurrentProcessId(),
        (static_cast<ULONGLONG>(created.dwHighDateTime) << 32) | created.dwLowDateTime);
    if (SendLine(pipe, request, deadline) && ReadLine(&reader, response, sizeof(response), deadline)
        && strncmp(response, challenge_prefix, strlen(challenge_prefix)) == 0) {
      const char* nonce = response + strlen(challenge_prefix);
      bool valid = strlen(nonce) == 36;
      for (size_t i = 0; valid && i < 36; ++i) valid = (nonce[i] >= '0' && nonce[i] <= '9') || (nonce[i] >= 'a' && nonce[i] <= 'f') || nonce[i] == '-';
      if (valid) {
        std::string proof = "RUNTIME_RECONNECT_PROOF";
        proof += response + strlen("RUNTIME_RECONNECT_CHALLENGE");
        if (SendLine(pipe, proof, deadline) && ReadLine(&reader, response, sizeof(response), deadline)
            && strcmp(expected, response) == 0) result = ERROR_SUCCESS;
      }
    }
  }
  CloseHandle(pipe);
  return result;
}

int EnvBoxIpcNotifyRuntimeIdentity(HINSTANCE module, const int* counts,
                                    size_t count) {
  const RuntimeProfile* p = EnvBoxProfile();
  if (p == nullptr || counts == nullptr || count != 11) return 0;
  // Serialize actual immutable Runtime values, not a supplied snapshot token.
  auto wide = [](const wchar_t* value) {
    int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value, -1,
                                nullptr, 0, nullptr, nullptr);
    if (n <= 0) return std::string();
    std::string result(static_cast<size_t>(n), '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value, -1,
                        result.data(), n, nullptr, nullptr);
    result.resize(static_cast<size_t>(n - 1));
    return result;
  };
  std::string actual = "PROFILE";
  auto kvw = [&](const char* key, const wchar_t* value) {
    std::string text = wide(value); AppendKv(&actual, key, text.c_str());
  };
  kvw("profile_id", p->profile_id);
  kvw("instance_id", p->instance_id);
  kvw("locale_name", p->locale_name);
  kvw("ui_language", p->ui_language);
  kvw("region", p->region);
  kvw("tz_windows", p->tz_windows);
  kvw("tz_iana", p->tz_iana);
  AppendKvU32(&actual, "inherit_children", p->inherit_children ? 1 : 0);
  AppendKvU32(&actual, "audit", p->audit ? 1 : 0);
  kvw("webrtc", p->webrtc_policy);
  auto dns_field = [](void* context, const char* key, const char* value) -> int {
    auto* config = static_cast<std::string*>(context);
    // The shared emitter uses ENV mode names; PROFILE wire uses numeric flags.
    if (strcmp(key, "dns_mode") == 0) {
      if (strcmp(value, "host") == 0) AppendKv(config, key, "0");
      else if (strcmp(value, "virtual_view") == 0) AppendKv(config, key, "1");
      else return 0;
    } else if (strcmp(key, "dns_servers") == 0) {
      std::string servers = value; size_t start = 0;
      while (start < servers.size()) { size_t end = servers.find(';', start); if (end == std::string::npos) end = servers.size(); std::string server = servers.substr(start, end - start); AppendKv(config, "dns_server", server.c_str()); start = end + 1; }
    } else AppendKv(config, key, value);
    return config->size() < kMaxLine;
  };
  if (!EnvBoxEmitDnsConfiguration(p, dns_field, &actual) ||
      !EnvBoxEmitIdentityConfiguration(p, dns_field, &actual)) return 0;
  for (int i = 0; i < p->registry_path_count; ++i)
    kvw("registry_path", p->registry_paths[i]);
  bool complete = EnvBoxProfileEnvironmentComplete() != 0;
  for (int i = 0; i < p->environment_count; ++i) {
    kvw("environment", p->environment[i]);
    const wchar_t* eq = wcschr(p->environment[i], L'=');
    if (eq == nullptr || eq == p->environment[i]) { complete = false; continue; }
    std::wstring key(p->environment[i], eq - p->environment[i]);
    SetLastError(ERROR_SUCCESS);
    DWORD n = GetEnvironmentVariableW(key.c_str(), nullptr, 0);
    if (n == 0) {
      if (eq[1] != L'\0' || GetLastError() == ERROR_ENVVAR_NOT_FOUND)
        complete = false;
    } else {
      std::wstring value(n, L'\0');
      DWORD got = GetEnvironmentVariableW(key.c_str(), value.data(), n);
      if (got >= n || got == 0) { complete = false; continue; }
      value.resize(got);
      if (value != eq + 1) complete = false;
    }
  }
  FILETIME created = {}, exited = {}, kernel = {}, user = {};
  if (!GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user)) return 0;
  ULONGLONG generation = (static_cast<ULONGLONG>(created.dwHighDateTime) << 32)
                         | created.dwLowDateTime;
  wchar_t path[32768] = {};
  DWORD length = GetModuleFileNameW(module, path, 32768);
  if (length == 0 || length >= 32768) return 0;
  std::string body;
  AppendKvU32(&body, "pid", GetCurrentProcessId());
  char number[32];
  _snprintf_s(number, sizeof(number), _TRUNCATE, "%llu", generation);
  AppendKv(&body, "creation_time", number);
  AppendKvU32(&body, "protocol", 1);
  AppendKv(&body, "version", ENVBOX_RUNTIME_VERSION);
  std::string path_utf8 = wide(path);
  AppendKv(&body, "module_path", path_utf8.c_str());
  AppendKv(&body, "actual_profile", actual.c_str());
  AppendKvU32(&body, "config_complete", complete ? 1 : 0);
  const char* groups[] = {"time", "winrt_time", "geo", "locale", "crt_locale",
                          "language", "dns", "registry", "process", "network_policy", "identity"};
  for (size_t i = 0; i < count; ++i) {
    std::string entry = groups[i]; entry += ":"; entry += std::to_string(counts[i]);
    AppendKv(&body, "hook", entry.c_str());
  }
  if (body.size() + sizeof("RUNTIME_IDENTITY_CONFIRM") >= 32768) return 0;
  g_runtime_identity_wire = "RUNTIME_IDENTITY";
  g_runtime_identity_wire += body;
  // This remains a bounded notice; acceptance and application-entry release
  // are separate host decisions. Sending it is not an accepted ACK.
  // A PROFILE connection just closed; the serial broker can briefly have
  // no pipe instance while replacing it. A required identity must retry this
  // handoff rather than taking the legacy notice's immediate FILE_NOT_FOUND.
  wchar_t gate[8] = {};
  bool gated = GetEnvironmentVariableW(L"ENVBOX_STARTUP_GATE",gate,8)==1 && gate[0]==L'1';
  if (EnvBoxProfileEnvironmentComplete() && !gated) {
    // Short-lived legacy targets must not close before authentication or
    // application entry. Gate targets retain their outside-loader handshake.
    const ULONGLONG deadline=GetTickCount64()+3000;
    HANDLE pipe=ConnectPipe(RemainingMs(deadline));
    if (pipe==INVALID_HANDLE_VALUE) return 0;
    PipeReader reader={}; reader.h=pipe;
    char response[256]={}, expected[160];
    _snprintf_s(expected,sizeof(expected),_TRUNCATE,
        "RUNTIME_IDENTITY_CONFIRMED pid=%lu creation_time=%llu",GetCurrentProcessId(),generation);
    int accepted=SendLine(pipe,std::string("RUNTIME_IDENTITY_CONFIRM")+body,deadline) &&
        ReadLine(&reader,response,sizeof(response),deadline) && strcmp(expected,response)==0;
    CloseHandle(pipe);
    return accepted;
  }
  Notify("RUNTIME_IDENTITY", body, EnvBoxProfileEnvironmentComplete() ? 2000 : 100);
  return 1;
}

void EnvBoxIpcNotifyHookError(const char* api_utf8, unsigned long code,
                              const char* detail_utf8) {
  std::string body;
  AppendKvU32(&body, "pid", (unsigned long)GetCurrentProcessId());
  AppendKv(&body, "api", api_utf8 != nullptr ? api_utf8 : "");
  AppendKvU32(&body, "code", code);
  if (detail_utf8 != nullptr && detail_utf8[0] != '\0') {
    AppendKv(&body, "detail", detail_utf8);
  }
  Notify("HOOK_ERROR", body);
}

void EnvBoxIpcNotifyProcessCreated(unsigned long child_pid,
                                   const char* image_utf8) {
  std::string body;
  AppendKvU32(&body, "pid", (unsigned long)GetCurrentProcessId());
  AppendKvU32(&body, "child_pid", child_pid);
  if (image_utf8 != nullptr && image_utf8[0] != '\0') {
    AppendKv(&body, "image", image_utf8);
  }
  Notify("PROCESS_CREATED", body);
}

int EnvBoxIpcRegisterChild(HANDLE child, unsigned long child_pid) {
  const RuntimeProfile* profile = EnvBoxProfile();
  FILETIME parent_created = {}, child_created = {}, exited = {}, kernel = {}, user = {};
  if (!profile || GetProcessId(child) != child_pid ||
      !GetProcessTimes(GetCurrentProcess(), &parent_created, &exited, &kernel, &user) ||
      !GetProcessTimes(child, &child_created, &exited, &kernel, &user)) return 0;
  ULONGLONG parent_generation = (static_cast<ULONGLONG>(parent_created.dwHighDateTime) << 32) | parent_created.dwLowDateTime;
  ULONGLONG child_generation = (static_cast<ULONGLONG>(child_created.dwHighDateTime) << 32) | child_created.dwLowDateTime;
  ULONGLONG deadline = GetTickCount64() + 3000;
  HANDLE pipe = ConnectPipe(RemainingMs(deadline));
  if (pipe == INVALID_HANDLE_VALUE) { SetLastError(ERROR_TIMEOUT); return 0; }
  std::string request = "REGISTER_CHILD";
  AppendKvU32(&request, "pid", GetCurrentProcessId());
  AppendKvU32(&request, "child_pid", child_pid);
  char number[32], id[64];
  _snprintf_s(number, sizeof(number), _TRUNCATE, "%llu", parent_generation);
  AppendKv(&request, "creation_time", number);
  _snprintf_s(number, sizeof(number), _TRUNCATE, "%llu", child_generation);
  AppendKv(&request, "child_creation_time", number);
  WideToUtf8(profile->instance_id, id, sizeof(id)); AppendKv(&request, "instance_id", id);
  WideToUtf8(profile->profile_id, id, sizeof(id)); AppendKv(&request, "profile_id", id);
  char expected[128], response[256];
  _snprintf_s(expected, sizeof(expected), _TRUNCATE, "CHILD_BOUND pid=%lu creation_time=%llu", child_pid, child_generation);
  PipeReader reader = {}; reader.h = pipe;
  int ok = SendLine(pipe, request, deadline) && ReadLine(&reader, response, sizeof(response), deadline) && strcmp(expected, response) == 0;
  ClosePipe(pipe);
  if (!ok) SetLastError(ERROR_ACCESS_DENIED);
  return ok;
}

void EnvBoxIpcNotifyProcessExited(unsigned long exit_code) {
  EnvBoxIpcNotifyProcessExitedPid((unsigned long)GetCurrentProcessId(),
                                 exit_code);
}

void EnvBoxIpcNotifyProcessExitedPid(unsigned long pid,
                                    unsigned long exit_code) {
  std::string body;
  AppendKvU32(&body, "pid", pid);
  AppendKvU32(&body, "exit_code", exit_code);
  Notify("PROCESS_EXITED", body);
}
