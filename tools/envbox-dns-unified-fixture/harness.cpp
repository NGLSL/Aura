#include <winsock2.h>
#include <windows.h>
#include <windns.h>
#include <psapi.h>
#include <atomic>
#include <cstdio>
#include <string>
#include <thread>
#include <vector>

namespace {
struct Handle {
  HANDLE value = nullptr;
  explicit Handle(HANDLE v = nullptr) : value(v) {}
  ~Handle() { if (value) CloseHandle(value); }
  Handle(const Handle&) = delete;
  Handle& operator=(const Handle&) = delete;
};

bool RuntimeLoaded() {
  return GetModuleHandleW(L"envbox-runtime64.dll") || GetModuleHandleW(L"envbox-runtime32.dll");
}

std::wstring Environment(const wchar_t* key) {
  wchar_t value[128] = {};
  DWORD length = GetEnvironmentVariableW(key, value, 128);
  return length && length < 128 ? value : L"";
}

std::string JsonString(const std::wstring& value) {
  int length = WideCharToMultiByte(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
  std::string utf8(length, '\0');
  if (length) WideCharToMultiByte(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), utf8.data(), length, nullptr, nullptr);
  std::string result;
  for (char character : utf8) {
    if (character == '\\' || character == '"') result += '\\';
    result += character;
  }
  return result;
}

struct Sample {
  DWORD handles = 0;
  SIZE_T private_bytes = 0;
};

Sample Measure(const char* phase) {
  Sample sample;
  PROCESS_MEMORY_COUNTERS_EX counters = {};
  counters.cb = sizeof(counters);
  if (!GetProcessHandleCount(GetCurrentProcess(), &sample.handles) ||
      !GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&counters), sizeof(counters)))
    ExitProcess(20);
  sample.private_bytes = counters.PrivateUsage;
  std::printf("{\"phase\":\"%s\",\"pid\":%lu,\"handles\":%lu,\"private_bytes\":%llu}\n",
              phase, GetCurrentProcessId(), sample.handles, static_cast<unsigned long long>(sample.private_bytes));
  std::fflush(stdout);
  return sample;
}

bool Query(const wchar_t* name, bool fixture) {
  PDNS_RECORDW records = nullptr;
  DNS_STATUS status = DnsQuery_W(name, DNS_TYPE_TEXT, DNS_QUERY_BYPASS_CACHE | DNS_QUERY_NO_HOSTS_FILE,
                                 nullptr, &records, nullptr);
  bool valid = status == ERROR_SUCCESS && records && records->wType == DNS_TYPE_TEXT &&
               records->Data.TXT.dwStringCount > 0 && records->Data.TXT.pStringArray[0];
  if (valid && fixture)
    valid = wcscmp(records->Data.TXT.pStringArray[0], L"profile-marker") == 0;
  DnsRecordListFree(records, DnsFreeRecordList);
  if (!valid) std::printf("{\"query_failure\":%ld}\n", status);
  return valid;
}

struct Async {
  Handle event{CreateEventW(nullptr, TRUE, FALSE, nullptr)};
  std::atomic<unsigned> callbacks{0};
  DNS_QUERY_RESULT result{};
  Async() { result.Version = DNS_QUERY_RESULTS_VERSION1; }
};

void WINAPI Completed(void* pointer, DNS_QUERY_RESULT* result) {
  auto* context = static_cast<Async*>(pointer);
  context->callbacks.fetch_add(1);
  DnsRecordListFree(result->pQueryRecords, DnsFreeRecordList);
  result->pQueryRecords = nullptr;
  SetEvent(context->event.value); // Last access; caller keeps storage until this signal.
}

bool Cancel(const wchar_t* name) {
  Async context;
  if (!context.event.value) return false;
  DNS_QUERY_REQUEST request = {};
  request.Version = DNS_QUERY_REQUEST_VERSION1;
  request.QueryName = name;
  request.QueryType = DNS_TYPE_TEXT;
  request.QueryOptions = DNS_QUERY_BYPASS_CACHE | DNS_QUERY_NO_HOSTS_FILE;
  request.pQueryCompletionCallback = Completed;
  request.pQueryContext = &context;
  DNS_QUERY_CANCEL cancel = {};
  DNS_STATUS returned = DnsQueryEx(&request, &context.result, &cancel);
  if (returned != DNS_REQUEST_PENDING) {
    DnsRecordListFree(context.result.pQueryRecords, DnsFreeRecordList);
    return false;
  }
  Sleep(10);
  DNS_QUERY_CANCEL copy = cancel;
  DNS_STATUS first = DnsCancelQuery(&cancel);
  DNS_STATUS second = DnsCancelQuery(&copy);
  DWORD waited = WaitForSingleObject(context.event.value, 10000);
  if (waited != WAIT_OBJECT_0) {
    std::printf("{\"async_storage_timeout\":true}\n");
    std::fflush(stdout);
    ExitProcess(21); // Do not return while a callback may still borrow local storage.
  }
  DNS_STATUS completed = context.result.QueryStatus;
  unsigned callbacks = context.callbacks.load();
  std::printf("{\"cancel_first\":%ld,\"cancel_copy\":%ld,\"completion\":%ld,\"callbacks\":%u}\n",
              first, second, completed, callbacks);
  return first == ERROR_SUCCESS && (second == ERROR_SUCCESS || second == ERROR_INVALID_PARAMETER) &&
         completed == ERROR_CANCELLED && callbacks == 1;
}

unsigned Batch(const wchar_t* name, bool fixture, bool parallel) {
  std::atomic<unsigned> failures{0};
  auto batch = [&] { for (unsigned i = 0; i < 8; ++i) if (!Query(name, fixture)) ++failures; };
  if (parallel) {
    std::vector<std::thread> workers;
    for (unsigned i = 0; i < 4; ++i) workers.emplace_back(batch);
    for (auto& worker : workers) worker.join();
  } else {
    batch();
  }
  return failures.load();
}

bool Child(const wchar_t* name, bool fixture) {
  wchar_t exe[32768] = {};
  if (!GetModuleFileNameW(nullptr, exe, 32768)) return false;
  std::wstring command = L"\"" + std::wstring(exe) + L"\" --child \"" + name + L"\" " + (fixture ? L"fixture" : L"public") +
                         L" " + Environment(L"ENVBOX_PROFILE_ID") + L" " + Environment(L"ENVBOX_INSTANCE_ID");
  SECURITY_ATTRIBUTES security = {sizeof(security), nullptr, TRUE};
  HANDLE read_value = nullptr, write_value = nullptr;
  if (!CreatePipe(&read_value, &write_value, &security, 0)) return false;
  Handle read_pipe(read_value), write_pipe(write_value);
  if (!SetHandleInformation(read_pipe.value, HANDLE_FLAG_INHERIT, 0)) return false;
  STARTUPINFOW startup = {}; startup.cb = sizeof(startup);
  startup.dwFlags = STARTF_USESTDHANDLES;
  startup.hStdOutput = startup.hStdError = write_pipe.value;
  startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
  PROCESS_INFORMATION process = {};
  if (!CreateProcessW(exe, command.data(), nullptr, nullptr, TRUE, 0, nullptr, nullptr, &startup, &process)) {
    std::printf("{\"child_create_error\":%lu}\n", GetLastError());
    return false;
  }
  CloseHandle(write_pipe.value); write_pipe.value = nullptr;
  Handle owned_process(process.hProcess), owned_thread(process.hThread);
  DWORD wait = WaitForSingleObject(owned_process.value, 30000);
  if (wait != WAIT_OBJECT_0) {
    TerminateProcess(owned_process.value, 22);
    WaitForSingleObject(owned_process.value, 5000);
    return false;
  }
  DWORD code = 0;
  if (!GetExitCodeProcess(owned_process.value, &code)) return false;
  char buffer[4096]; DWORD count = 0;
  DWORD available = 0;
  while (PeekNamedPipe(read_pipe.value, nullptr, 0, nullptr, &available, nullptr) && available) {
    DWORD requested = available < sizeof(buffer) ? available : sizeof(buffer);
    if (!ReadFile(read_pipe.value, buffer, requested, &count, nullptr) || !count) break;
    std::fwrite(buffer, 1, count, stdout);
  }
  std::printf("{\"child_pid\":%lu,\"child_exit\":%lu}\n", process.dwProcessId, code);
  return code == 0;
}
} // namespace

int wmain(int argc, wchar_t** argv) {
  if (argc < 3 || !RuntimeLoaded()) return 2;
  bool fixture = argc > 3 && wcscmp(argv[3], L"fixture") == 0;
  std::wstring profile = Environment(L"ENVBOX_PROFILE_ID"), instance = Environment(L"ENVBOX_INSTANCE_ID");
  if (profile.empty() || instance.empty()) return 6;
  HMODULE runtime = GetModuleHandleW(L"envbox-runtime64.dll");
  if (!runtime) runtime = GetModuleHandleW(L"envbox-runtime32.dll");
  wchar_t module[32768] = {};
  if (!GetModuleFileNameW(runtime, module, 32768)) return 7;
  std::printf("{\"pid\":%lu,\"runtime_loaded\":true,\"runtime_path\":\"%s\",\"profile_id\":\"%s\",\"instance_id\":\"%s\"}\n",
              GetCurrentProcessId(), JsonString(module).c_str(), JsonString(profile).c_str(), JsonString(instance).c_str());
  if (wcscmp(argv[1], L"--child") == 0) {
    if (argc != 6 || profile != argv[4] || instance != argv[5]) return 8;
    return Batch(argv[2], fixture, false) ? 3 : 0;
  }
  if (wcscmp(argv[1], L"--cancel") == 0) {
    unsigned failed = 0;
    for (unsigned i = 0; i < 4; ++i) if (!Cancel(argv[2])) ++failed;
    Sample before = Measure("cancel-warm");
    for (unsigned i = 0; i < 32; ++i) if (!Cancel(argv[2])) ++failed;
    Sleep(100);
    Sample after = Measure("cancel-end");
    bool stable = after.handles <= before.handles + 4 && after.private_bytes <= before.private_bytes + 2 * 1024 * 1024;
    std::printf("{\"cancel_queries\":36,\"failures\":%u,\"stable\":%s}\n", failed, stable ? "true" : "false");
    return failed || !stable ? 4 : 0;
  }
  unsigned failed = Batch(argv[2], fixture, false);
  Measure("warm");
  failed += Batch(argv[2], fixture, false);
  Measure("repeat-end");
  failed += Batch(argv[2], fixture, true);
  Sample first = Measure("concurrent-first");
  failed += Batch(argv[2], fixture, true);
  Sample last = Measure("concurrent-second");
  bool stable = last.handles <= first.handles + 4 && last.private_bytes <= first.private_bytes + 2 * 1024 * 1024;
  bool child = Child(argv[2], fixture);
  std::printf("{\"queries\":80,\"failures\":%u,\"stable\":%s,\"child_pass\":%s}\n", failed,
              stable ? "true" : "false", child ? "true" : "false");
  return failed || !stable || !child ? 5 : 0;
}
