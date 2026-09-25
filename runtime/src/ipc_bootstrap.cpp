// Named Pipe client for IPC Bootstrap (ticket 40). Protocol: see ipc_bootstrap.h.
// No third-party libs: Win32 CreateFileW / ReadFile / WriteFile only.
// All failures are contained here (return 0 / no-op). Startup Fail Policy is
// enforced by the caller (EnvBoxLoadProfile), not by this module.

#include "ipc_bootstrap.h"

#include <stdio.h>
#include <string.h>

#include <string>

namespace {

constexpr DWORD kConnectTimeoutMs = 2000;
constexpr DWORD kIoTimeoutMs = 3000;
constexpr size_t kMaxLine = 8192;
constexpr int kMaxKv = 48;

// ---------------------------------------------------------------------------
// Pipe name resolution: ENVBOX_IPC_PIPE (bare name or \\.\pipe\... path).
// Default: \\.\pipe\envbox-runtime.
// ---------------------------------------------------------------------------
void GetPipePath(wchar_t* out, size_t cap) {
  DWORD n = GetEnvironmentVariableW(L"ENVBOX_IPC_PIPE", out, (DWORD)cap);
  if (n == 0 || n >= cap) {
    wcsncpy_s(out, cap, L"\\\\.\\pipe\\envbox-runtime", _TRUNCATE);
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
        CancelIo(h);
        CloseHandle(ov.hEvent);
        return 0;
      }
      if (!GetOverlappedResult(h, &ov, &n, FALSE)) {
        CloseHandle(ov.hEvent);
        return 0;
      }
    }
    CloseHandle(ov.hEvent);
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
      CancelIo(r->h);
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
HANDLE ConnectPipe(void) {
  wchar_t path[256];
  GetPipePath(path, 256);
  ULONGLONG deadline = GetTickCount64() + kConnectTimeoutMs;
  for (;;) {
    // Wait briefly for an instance; ignore failure and still try CreateFileW.
    WaitNamedPipeW(path, 100);
    HANDLE h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                           OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
    if (h != INVALID_HANDLE_VALUE) {
      DWORD mode = PIPE_READMODE_BYTE;
      SetNamedPipeHandleState(h, &mode, nullptr, nullptr);
      return h;
    }
    if (RemainingMs(deadline) == 0) {
      char msg[128];
      _snprintf_s(msg, sizeof(msg), _TRUNCATE,
                  "EnvBox IPC: connect failed (GetLastError=%lu)\n",
                  (unsigned long)GetLastError());
      OutputDebugStringA(msg);
      return INVALID_HANDLE_VALUE;
    }
    Sleep(50);
  }
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
  char val[kMaxKv][512];
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
  return o > 0 || cap > 0;
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
      UnquoteInto(line + start, i - start, out->val[out->count],
                  sizeof(out->val[0]));
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

int MsgGetAllW(const IpcMsg* m, const char* key, wchar_t out[][128],
               int max_items, size_t item_cap) {
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

// Apply a decoded PROFILE message into *out. Returns 1 when required fields
// are present (same completeness rule as the profiles.toml path).
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
  out->dns_mode = ParseFlag(m, "dns_mode", 0);
  out->dns_server_count =
      MsgGetAll(m, "dns_server", out->dns_servers, ENVBOX_DNS_MAX, 64);
  if (out->dns_mode == 1 && out->dns_server_count == 0) {
    // Same rule as the TOML path: VirtualView without servers -> Host.
    out->dns_mode = 0;
  }
  out->registry_path_count =
      MsgGetAllW(m, "registry_path", out->registry_paths, ENVBOX_REG_MAX, 128);

  return out->has_locale && out->has_ui && out->has_region && out->has_tz;
}

// Send one fire-and-forget message on a fresh connection. Best-effort.
void Notify(const char* name, const std::string& body) {
  HANDLE h = ConnectPipe();
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

  HANDLE h = ConnectPipe();
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
  IpcMsg msg;
  int got = 0;
  for (;;) {
    if (!ReadLine(&reader, line, sizeof(line), deadline)) {
      OutputDebugStringA("EnvBox IPC: PROFILE read failed (timeout/eof)\n");
      ClosePipe(h);
      return 0;
    }
    if (!ParseMsg(line, &msg)) {
      OutputDebugStringA("EnvBox IPC: malformed message ignored\n");
      continue;
    }
    if (strcmp(msg.name, "PROFILE") == 0) {
      if (!FillProfileFromMsg(&msg, out)) {
        OutputDebugStringA("EnvBox IPC: PROFILE incomplete\n");
        ClosePipe(h);
        return 0;
      }
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

void EnvBoxIpcNotifyProcessExited(unsigned long exit_code) {
  std::string body;
  AppendKvU32(&body, "pid", (unsigned long)GetCurrentProcessId());
  AppendKvU32(&body, "exit_code", exit_code);
  Notify("PROCESS_EXITED", body);
}
