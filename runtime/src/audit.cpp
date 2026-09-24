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
static SRWLOCK g_lock = SRWLOCK_INIT;

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
    _snwprintf_s(root, MAX_PATH, _TRUNCATE, L"%s\\EnvBox", appdata);
  } else if (n >= MAX_PATH) {
    // Explicit CONFIG_ROOT that does not fit: do not fall back to another root.
    return;  // Fail Open
  }

  wchar_t dir[MAX_PATH + 16];
  _snwprintf_s(dir, MAX_PATH + 16, _TRUNCATE, L"%s\\audit", root);
  CreateDirectoryW(dir, nullptr);

  wchar_t path[MAX_PATH + 80];
  _snwprintf_s(path, MAX_PATH + 80, _TRUNCATE, L"%s\\%s.jsonl", dir,
               pfl->instance_id);

  g_audit = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ, nullptr,
                        OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (g_audit == INVALID_HANDLE_VALUE) {
    return;  // Fail Open
  }
  g_audit_on = 1;
  g_pid = GetCurrentProcessId();
  // ppid resolved lazily on first event (avoid Toolhelp under loader lock).
  EnvBoxAuditEvent("EnvBoxAuditInit", 0, "schema=v1");
}

void EnvBoxAuditEvent(const char* api, int virtualized, const char* summary) {
  if (!g_audit_on || api == nullptr) {
    return;
  }
  AcquireSRWLockExclusive(&g_lock);
  if (g_audit == INVALID_HANDLE_VALUE) {
    ReleaseSRWLockExclusive(&g_lock);
    return;
  }
  if (g_ppid == 0) {
    g_ppid = EnvBoxParentPid();
  }
  char ts[40];
  FormatUtc(ts, sizeof(ts));
  char api_esc[128];
  JsonEscape(api, api_esc, sizeof(api_esc));
  char line[512];
  int len;
  if (summary != nullptr && summary[0] != '\0') {
    char esc[192];
    JsonEscape(summary, esc, sizeof(esc));
    len = _snprintf_s(
        line, sizeof(line), _TRUNCATE,
        "{\"v\":1,\"ts_utc\":\"%s\",\"pid\":%lu,\"ppid\":%lu,\"tid\":%lu,"
        "\"api\":\"%s\",\"virtualized\":%s,\"summary\":\"%s\"}\n",
        ts, (unsigned long)g_pid, (unsigned long)g_ppid,
        (unsigned long)GetCurrentThreadId(), api_esc,
        virtualized ? "true" : "false", esc);
  } else {
    len = _snprintf_s(
        line, sizeof(line), _TRUNCATE,
        "{\"v\":1,\"ts_utc\":\"%s\",\"pid\":%lu,\"ppid\":%lu,\"tid\":%lu,"
        "\"api\":\"%s\",\"virtualized\":%s}\n",
        ts, (unsigned long)g_pid, (unsigned long)g_ppid,
        (unsigned long)GetCurrentThreadId(), api_esc,
        virtualized ? "true" : "false");
  }
  if (len > 0) {
    DWORD written = 0;
    WriteFile(g_audit, line, (DWORD)len, &written, nullptr);
    FlushFileBuffers(g_audit);
  }
  ReleaseSRWLockExclusive(&g_lock);
}

void EnvBoxAuditShutdown(void) {
  AcquireSRWLockExclusive(&g_lock);
  if (g_audit != INVALID_HANDLE_VALUE) {
    CloseHandle(g_audit);
    g_audit = INVALID_HANDLE_VALUE;
  }
  g_audit_on = 0;
  ReleaseSRWLockExclusive(&g_lock);
}
