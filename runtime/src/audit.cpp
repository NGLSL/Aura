// Audit Mode JSONL sink (ticket 20). Fail Open: I/O errors never break hooks.
// Wide paths for non-ASCII user profile dirs. Schema matches envbox-core::AuditEvent v1.
// Enable source is immutable RuntimeProfile only (loaded from ENVBOX_AUDIT == "1").

#include "audit.h"

#include <stdio.h>
#include <string.h>

#include <tlhelp32.h>

static HANDLE g_audit = INVALID_HANDLE_VALUE;
static int g_audit_on = 0;
static DWORD g_pid = 0;
static DWORD g_ppid = 0;  // 0 = unknown until first event (avoid Toolhelp under DllMain)
static char g_image[64] = {0};  // process basename, UTF-8
static SRWLOCK g_lock = SRWLOCK_INIT;
// Flush is expensive (FlushFileBuffers). Buffer JSONL lines and WriteFile in
// batches; flush on threshold/interval and at shutdown so a crash can lose at
// most a few events. Typing/IME under Audit Mode fires locale/registry hooks
// in tight loops - one WriteFile per event was a visible stutter source.
static char g_linebuf[16 * 1024];
static size_t g_linebuf_len = 0;
static unsigned g_unflushed = 0;
static ULONGLONG g_last_flush = 0;
static const unsigned kAuditFlushEvery = 64;
static const DWORD kAuditFlushIntervalMs = 1000;

// Short-window collapse: identical (api, virtualized, summary) bursts from
// IME/typing become one JSONL line with "n". Key change or flush ends the
// pending line. Cap keeps a single burst from holding forever.
static const unsigned kAuditCollapseMax = 4096;
struct AuditPending {
  int active;
  int virtualized;
  unsigned n;
  unsigned long tid;
  char api[80];
  char summary[192];
  char ts[40];
};
static AuditPending g_pending;

// Defined below; used when emitting collapsed lines.
static void JsonEscape(const char* src, char* dst, size_t cap);

static void AuditAppendLineLocked(const char* line, int len) {
  if (len <= 0) return;
  if (g_linebuf_len + (size_t)len > sizeof(g_linebuf)) {
    if (g_audit != INVALID_HANDLE_VALUE && g_linebuf_len > 0) {
      DWORD written = 0;
      WriteFile(g_audit, g_linebuf, (DWORD)g_linebuf_len, &written, nullptr);
    }
    g_linebuf_len = 0;
  }
  if (g_linebuf_len + (size_t)len <= sizeof(g_linebuf)) {
    memcpy(g_linebuf + g_linebuf_len, line, (size_t)len);
    g_linebuf_len += (size_t)len;
    g_unflushed++;
  }
}

static void AuditEmitPendingLocked(void) {
  if (!g_pending.active) return;
  char api_esc[128];
  char sum_esc[192];
  char image_esc[80];
  JsonEscape(g_pending.api, api_esc, sizeof(api_esc));
  JsonEscape(g_pending.summary, sum_esc, sizeof(sum_esc));
  JsonEscape(g_image, image_esc, sizeof(image_esc));
  char line[700];
  int len;
  if (g_pending.summary[0] != '\0') {
    len = _snprintf_s(
        line, sizeof(line), _TRUNCATE,
        "{\"v\":1,\"ts_utc\":\"%s\",\"pid\":%lu,\"ppid\":%lu,\"tid\":%lu,"
        "\"api\":\"%s\",\"virtualized\":%s,\"summary\":\"%s\",\"image\":\"%s\",\"n\":%u}\n",
        g_pending.ts, (unsigned long)g_pid, (unsigned long)g_ppid,
        g_pending.tid, api_esc, g_pending.virtualized ? "true" : "false",
        sum_esc, image_esc, g_pending.n);
  } else {
    len = _snprintf_s(
        line, sizeof(line), _TRUNCATE,
        "{\"v\":1,\"ts_utc\":\"%s\",\"pid\":%lu,\"ppid\":%lu,\"tid\":%lu,"
        "\"api\":\"%s\",\"virtualized\":%s,\"image\":\"%s\",\"n\":%u}\n",
        g_pending.ts, (unsigned long)g_pid, (unsigned long)g_ppid,
        g_pending.tid, api_esc, g_pending.virtualized ? "true" : "false",
        image_esc, g_pending.n);
  }
  AuditAppendLineLocked(line, len);
  g_pending.active = 0;
  g_pending.n = 0;
}

static void AuditFlushLocked(void) {
  AuditEmitPendingLocked();
  if (g_audit == INVALID_HANDLE_VALUE) {
    g_linebuf_len = 0;
    g_unflushed = 0;
    return;
  }
  if (g_linebuf_len > 0) {
    DWORD written = 0;
    WriteFile(g_audit, g_linebuf, (DWORD)g_linebuf_len, &written, nullptr);
    g_linebuf_len = 0;
  }
  FlushFileBuffers(g_audit);
  g_unflushed = 0;
  g_last_flush = GetTickCount64();
}

// Process image basename (e.g. chrome.exe) for per-software audit grouping.
static void EnvBoxCurrentImageA(char* out, size_t cap) {
  if (out == nullptr || cap == 0) {
    return;
  }
  out[0] = '\0';
  wchar_t path[MAX_PATH] = {};
  DWORD n = GetModuleFileNameW(nullptr, path, MAX_PATH);
  if (n == 0 || n >= MAX_PATH) {
    return;
  }
  const wchar_t* base = path;
  for (const wchar_t* p = path; *p; ++p) {
    if (*p == L'\\' || *p == L'/') {
      base = p + 1;
    }
  }
  WideCharToMultiByte(CP_UTF8, 0, base, -1, out, (int)cap, nullptr, nullptr);
}

// RAII + save GetLastError for the process snapshot HANDLE (AGENTS.md).
static DWORD EnvBoxParentPid(void) {
  DWORD pid = GetCurrentProcessId();
  HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
  if (snap == INVALID_HANDLE_VALUE) {
    DWORD err = GetLastError();
    (void)err;
    return 0;
  }
  PROCESSENTRY32W pe = {};
  pe.dwSize = sizeof(pe);
  DWORD ppid = 0;
  if (!Process32FirstW(snap, &pe)) {
    DWORD err = GetLastError();
    CloseHandle(snap);
    SetLastError(err);
    return 0;
  }
  do {
    if (pe.th32ProcessID == pid) {
      ppid = pe.th32ParentProcessID;
      break;
    }
  } while (Process32NextW(snap, &pe));
  CloseHandle(snap);
  return ppid;
}

static void FormatUtc(char* out, size_t cap) {
  SYSTEMTIME st = {};
  GetSystemTime(&st);
  _snprintf_s(out, cap, _TRUNCATE, "%04u-%02u-%02uT%02u:%02u:%02u.%03uZ",
              (unsigned)st.wYear, (unsigned)st.wMonth, (unsigned)st.wDay,
              (unsigned)st.wHour, (unsigned)st.wMinute, (unsigned)st.wSecond,
              (unsigned)st.wMilliseconds);
}

// JSON string escape. Keeps UTF-8 bytes; only quotes, backslash, and controls.
static void JsonEscape(const char* src, char* dst, size_t cap) {
  size_t o = 0;
  if (cap == 0) {
    return;
  }
  for (size_t i = 0; src && src[i] && o + 8 < cap; i++) {
    unsigned char c = (unsigned char)src[i];
    if (c == '"' || c == '\\') {
      dst[o++] = '\\';
      dst[o++] = (char)c;
    } else if (c < 0x20) {
      o += (size_t)_snprintf_s(dst + o, cap - o, _TRUNCATE, "\\u%04x", c);
    } else {
      dst[o++] = (char)c;
    }
  }
  dst[o] = '\0';
}

void EnvBoxAuditInit(const RuntimeProfile* pfl) {
  if (g_audit_on) {
    return;
  }
  // Single source of truth: RuntimeProfile.audit (set from ENVBOX_AUDIT == "1").
  if (pfl == nullptr || !pfl->audit) {
    return;
  }
  // Per-instance only: never share a global/unknown file.
  if (pfl->instance_id[0] == L'\0') {
    return;  // Fail Open
  }

  wchar_t root[MAX_PATH] = {};
  DWORD n = GetEnvironmentVariableW(L"ENVBOX_CONFIG_ROOT", root, MAX_PATH);
  if (n == 0) {
    wchar_t appdata[MAX_PATH] = {};
    if (GetEnvironmentVariableW(L"LOCALAPPDATA", appdata, MAX_PATH) == 0) {
      return;  // Fail Open: no sink
    }
    // Keep in sync with envbox-storage DATA_DIR_NAME (com.aura.envbox).
    _snwprintf_s(root, MAX_PATH, _TRUNCATE, L"%s\\com.aura.envbox", appdata);
    // One-time rename from the pre-0.3 `%LOCALAPPDATA%\EnvBox` layout.
    wchar_t legacy[MAX_PATH] = {};
    _snwprintf_s(legacy, MAX_PATH, _TRUNCATE, L"%s\\EnvBox", appdata);
    if (GetFileAttributesW(root) == INVALID_FILE_ATTRIBUTES &&
        GetFileAttributesW(legacy) != INVALID_FILE_ATTRIBUTES) {
      MoveFileW(legacy, root);  // best-effort
    }
  } else if (n >= MAX_PATH) {
    // Explicit CONFIG_ROOT that does not fit: do not fall back to another root.
    return;  // Fail Open
  }

  wchar_t dir[MAX_PATH + 16];
  _snwprintf_s(dir, MAX_PATH + 16, _TRUNCATE, L"%s\\audit", root);
  CreateDirectoryW(root, nullptr);
  CreateDirectoryW(dir, nullptr);

  wchar_t path[MAX_PATH + 80];
  _snwprintf_s(path, MAX_PATH + 80, _TRUNCATE, L"%s\\%s.jsonl", dir,
               pfl->instance_id);

  // Share read+write so the Process Tree Instance can append to one file.
  g_audit = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                        nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (g_audit == INVALID_HANDLE_VALUE) {
    return;  // Fail Open
  }
  g_audit_on = 1;
  g_pid = GetCurrentProcessId();
  EnvBoxCurrentImageA(g_image, sizeof(g_image));
  // ppid resolved lazily on first event (avoid Toolhelp under loader lock).
  EnvBoxAuditEvent("EnvBoxAuditInit", 0, "schema=v1");
}

void EnvBoxAuditEvent(const char* api, int virtualized, const char* summary) {
  if (!g_audit_on || api == nullptr) {
    return;
  }
  // Preserve GetLastError across audit I/O (AGENTS.md; CreateProcessW contract).
  DWORD last_err = GetLastError();
  AcquireSRWLockExclusive(&g_lock);
  if (g_audit == INVALID_HANDLE_VALUE) {
    ReleaseSRWLockExclusive(&g_lock);
    SetLastError(last_err);
    return;
  }
  if (g_ppid == 0) {
    g_ppid = EnvBoxParentPid();
  }

  const char* sum = (summary != nullptr && summary[0] != '\0') ? summary : "";
  // Collapse identical consecutive calls (typing / IME / registry loops).
  if (g_pending.active && g_pending.virtualized == virtualized &&
      g_pending.n < kAuditCollapseMax && strcmp(g_pending.api, api) == 0 &&
      strcmp(g_pending.summary, sum) == 0) {
    g_pending.n++;
    // Periodic end-of-window write so a long burst still splits into lines.
    if ((g_pending.n & 0x3f) == 0) {
      ULONGLONG now = GetTickCount64();
      if (now - g_last_flush >= kAuditFlushIntervalMs) {
        AuditFlushLocked();
      }
    }
    ReleaseSRWLockExclusive(&g_lock);
    SetLastError(last_err);
    return;
  }
  AuditEmitPendingLocked();

  // Start a new pending line (format JSON only when the key changes).
  g_pending.active = 1;
  g_pending.virtualized = virtualized;
  g_pending.n = 1;
  g_pending.tid = (unsigned long)GetCurrentThreadId();
  strncpy_s(g_pending.api, api, _TRUNCATE);
  strncpy_s(g_pending.summary, sum, _TRUNCATE);
  FormatUtc(g_pending.ts, sizeof(g_pending.ts));

  ULONGLONG now = GetTickCount64();
  if (g_unflushed >= kAuditFlushEvery ||
      now - g_last_flush >= kAuditFlushIntervalMs) {
    AuditFlushLocked();
  }
  ReleaseSRWLockExclusive(&g_lock);
  SetLastError(last_err);
}

void EnvBoxAuditEventW(const char* api, int virtualized, const wchar_t* summary) {
  if (!g_audit_on) {
    EnvBoxAuditEvent(api, virtualized, nullptr);
    return;
  }
  DWORD last_err = GetLastError();
  if (summary == nullptr) {
    EnvBoxAuditEvent(api, virtualized, nullptr);
    SetLastError(last_err);
    return;
  }
  char utf8[192];
  utf8[0] = '\0';
  int n = WideCharToMultiByte(CP_UTF8, 0, summary, -1, utf8, (int)sizeof(utf8),
                              nullptr, nullptr);
  if (n <= 0) {
    // Truncate rather than drop the whole summary.
    wchar_t clipped[96];
    wcsncpy_s(clipped, summary, _TRUNCATE);
    WideCharToMultiByte(CP_UTF8, 0, clipped, -1, utf8, (int)sizeof(utf8),
                        nullptr, nullptr);
  }
  EnvBoxAuditEvent(api, virtualized, utf8);
  SetLastError(last_err);
}

void EnvBoxAuditShutdown(void) {
  AcquireSRWLockExclusive(&g_lock);
  if (g_audit != INVALID_HANDLE_VALUE) {
    AuditFlushLocked();
    CloseHandle(g_audit);
    g_audit = INVALID_HANDLE_VALUE;
    g_unflushed = 0;
    g_linebuf_len = 0;
  }
  g_audit_on = 0;
  ReleaseSRWLockExclusive(&g_lock);
}
