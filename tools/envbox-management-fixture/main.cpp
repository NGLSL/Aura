#include <windows.h>

#include <sddl.h>

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

namespace {

constexpr DWORD kLowIntegrityRid = 0x1000;
constexpr DWORD kWaitMilliseconds = 10'000;

class ScopedHandle {
 public:
  ScopedHandle() = default;
  explicit ScopedHandle(HANDLE value) : value_(value) {}
  ScopedHandle(const ScopedHandle&) = delete;
  ScopedHandle& operator=(const ScopedHandle&) = delete;
  ScopedHandle(ScopedHandle&& other) noexcept : value_(other.release()) {}
  ScopedHandle& operator=(ScopedHandle&& other) noexcept {
    if (this != &other) reset(other.release());
    return *this;
  }
  ~ScopedHandle() { reset(); }

  HANDLE get() const { return value_; }
  bool valid() const { return value_ != nullptr && value_ != INVALID_HANDLE_VALUE; }
  void reset(HANDLE value = nullptr) {
    if (valid()) CloseHandle(value_);
    value_ = value;
  }
  HANDLE release() {
    HANDLE value = value_;
    value_ = nullptr;
    return value;
  }

 private:
  HANDLE value_ = nullptr;
};

std::wstring quote(const std::wstring& value) {
  std::wstring result = L"\"";
  for (wchar_t character : value) {
    if (character == L'\"') result += L'\"';
    result += character;
  }
  result += L"\"";
  return result;
}

std::wstring module_path() {
  std::vector<wchar_t> buffer(32'768);
  DWORD length = GetModuleFileNameW(nullptr, buffer.data(), static_cast<DWORD>(buffer.size()));
  if (length == 0 || length == buffer.size()) return {};
  return std::wstring(buffer.data(), length);
}

std::wstring last_error(DWORD error) {
  std::wostringstream stream;
  stream << error;
  return stream.str();
}

std::wstring token_sid(HANDLE token) {
  DWORD size = 0;
  GetTokenInformation(token, TokenUser, nullptr, 0, &size);
  if (size == 0) return {};
  std::vector<BYTE> data(size);
  if (!GetTokenInformation(token, TokenUser, data.data(), size, &size)) return {};
  auto* user = reinterpret_cast<TOKEN_USER*>(data.data());
  LPWSTR text = nullptr;
  if (!ConvertSidToStringSidW(user->User.Sid, &text)) return {};
  std::wstring result(text);
  LocalFree(text);
  return result;
}

DWORD token_integrity(HANDLE token) {
  DWORD size = 0;
  GetTokenInformation(token, TokenIntegrityLevel, nullptr, 0, &size);
  if (size == 0) return 0;
  std::vector<BYTE> data(size);
  if (!GetTokenInformation(token, TokenIntegrityLevel, data.data(), size, &size)) return 0;
  auto* label = reinterpret_cast<TOKEN_MANDATORY_LABEL*>(data.data());
  DWORD count = *GetSidSubAuthorityCount(label->Label.Sid);
  if (count == 0) return 0;
  return *GetSidSubAuthority(label->Label.Sid, count - 1);
}

bool write_pipe(HANDLE pipe, const std::string& line) {
  DWORD written = 0;
  return WriteFile(pipe, line.data(), static_cast<DWORD>(line.size()), &written, nullptr) &&
         written == line.size();
}

bool write_line(HANDLE pipe, const std::string& key, const std::string& value) {
  return write_pipe(pipe, key + "=" + value + "\n");
}

std::string narrow(const std::wstring& value) {
  if (value.empty()) return {};
  int length = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
                                  static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
  if (length <= 0) return {};
  std::string result(length, '\0');
  WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
                      static_cast<int>(value.size()), result.data(), length, nullptr, nullptr);
  return result;
}

int probe(const std::wstring& endpoint, const std::wstring& expected_sid, uintptr_t pipe_value) {
  ScopedHandle output(reinterpret_cast<HANDLE>(pipe_value));
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) {
    write_line(output.get(), "probe_token_error", narrow(last_error(GetLastError())));
    return 20;
  }
  ScopedHandle token_handle(token);
  std::wstring sid = token_sid(token_handle.get());
  DWORD integrity = token_integrity(token_handle.get());
  write_line(output.get(), "probe_pid", std::to_string(GetCurrentProcessId()));
  write_line(output.get(), "probe_sid", narrow(sid));
  write_line(output.get(), "probe_integrity", std::to_string(integrity));
  write_line(output.get(), "same_sid", sid == expected_sid ? "true" : "false");
  write_line(output.get(), "target_endpoint", narrow(endpoint));

  ScopedHandle pipe(CreateFileW(endpoint.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                                OPEN_EXISTING, 0, nullptr));
  if (!pipe.valid()) {
    DWORD error = GetLastError();
    write_line(output.get(), "pipe_open",
               error == ERROR_ACCESS_DENIED ? "denied" : "error");
    write_line(output.get(), "pipe_open_error_code", std::to_string(error));
    write_line(output.get(), "pipe_open_error", narrow(last_error(error)));
    return error == ERROR_ACCESS_DENIED ? 0 : 29;
  }
  DWORD server_pid = 0;
  if (GetNamedPipeServerProcessId(pipe.get(), &server_pid)) {
    write_line(output.get(), "server_pid", std::to_string(server_pid));
  } else {
    write_line(output.get(), "server_pid_error", narrow(last_error(GetLastError())));
  }
  const std::string request =
      "{\"version\":1,\"request_id\":\"00000000-0000-0000-0000-000000000001\","
      "\"command\":\"Ping\",\"run\":null,\"container_id\":null}\n";
  if (!write_pipe(pipe.get(), request)) {
    write_line(output.get(), "request_write", "failed");
    write_line(output.get(), "request_write_error", narrow(last_error(GetLastError())));
    return 0;
  }
  write_line(output.get(), "request_write", "ok");
  std::string response;
  char buffer[4096];
  for (;;) {
    DWORD received = 0;
    if (!ReadFile(pipe.get(), buffer, sizeof(buffer), &received, nullptr)) {
      DWORD error = GetLastError();
      write_line(output.get(), "response_read_error", narrow(last_error(error)));
      break;
    }
    response.append(buffer, buffer + received);
    if (response.find('\n') != std::string::npos || received == 0) break;
    if (response.size() > 65'536) break;
  }
  if (response.find("\"status\":\"AuthenticationDenied\"") != std::string::npos) {
    write_line(output.get(), "response_status", "AuthenticationDenied");
  } else if (response.find("\"status\":\"Ok\"") != std::string::npos) {
    write_line(output.get(), "response_status", "Ok");
  } else {
    write_line(output.get(), "response_status", "Other");
  }
  return 0;
}

int launch_low(const std::wstring& endpoint, const std::wstring& output_path) {
  HANDLE current_token_raw = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT |
                                                   TOKEN_ASSIGN_PRIMARY,
                        &current_token_raw)) {
    std::ofstream output{std::filesystem::path(output_path)};
    output << "stage=open_current_token\nerror=" << GetLastError() << "\n";
    return 21;
  }
  ScopedHandle current_token(current_token_raw);
  std::wstring sid = token_sid(current_token.get());
  DWORD parent_integrity = token_integrity(current_token.get());

  HANDLE restricted_raw = nullptr;
  if (!CreateRestrictedToken(current_token.get(), DISABLE_MAX_PRIVILEGE, 0, nullptr, 0, nullptr,
                             0, nullptr, &restricted_raw)) {
    DWORD error = GetLastError();
    std::ofstream output{std::filesystem::path(output_path)};
    output << "stage=create_restricted_token\nerror=" << error << "\n";
    return 22;
  }
  ScopedHandle restricted(restricted_raw);

  SID_IDENTIFIER_AUTHORITY mandatory_authority = SECURITY_MANDATORY_LABEL_AUTHORITY;
  PSID low_sid = nullptr;
  if (!AllocateAndInitializeSid(&mandatory_authority, 1, kLowIntegrityRid, 0, 0, 0, 0, 0, 0, 0,
                                &low_sid)) {
    DWORD error = GetLastError();
    std::ofstream output{std::filesystem::path(output_path)};
    output << "stage=allocate_low_sid\nerror=" << error << "\n";
    return 23;
  }
  TOKEN_MANDATORY_LABEL label{};
  label.Label.Attributes = SE_GROUP_INTEGRITY;
  label.Label.Sid = low_sid;
  if (!SetTokenInformation(restricted.get(), TokenIntegrityLevel, &label,
                           sizeof(TOKEN_MANDATORY_LABEL) + GetLengthSid(low_sid))) {
    DWORD error = GetLastError();
    FreeSid(low_sid);
    std::ofstream output{std::filesystem::path(output_path)};
    output << "stage=set_low_integrity\nerror=" << error << "\n";
    return 24;
  }
  FreeSid(low_sid);

  SECURITY_ATTRIBUTES attributes{};
  attributes.nLength = sizeof(attributes);
  attributes.bInheritHandle = TRUE;
  HANDLE read_pipe_raw = nullptr;
  HANDLE write_pipe_raw = nullptr;
  if (!CreatePipe(&read_pipe_raw, &write_pipe_raw, &attributes, 0)) {
    DWORD error = GetLastError();
    std::ofstream output{std::filesystem::path(output_path)};
    output << "stage=create_output_pipe\nerror=" << error << "\n";
    return 25;
  }
  ScopedHandle read_pipe(read_pipe_raw);
  ScopedHandle write_pipe_handle(write_pipe_raw);
  if (!SetHandleInformation(read_pipe.get(), HANDLE_FLAG_INHERIT, 0)) {
    DWORD error = GetLastError();
    std::ofstream output(narrow(output_path));
    output << "stage=make_output_read_end_noninheritable\nerror=" << error << "\n";
    return 26;
  }

  std::wstring path = module_path();
  std::wstring command = quote(path) + L" --probe " + quote(endpoint) + L" " + quote(sid) +
                         L" " + std::to_wstring(reinterpret_cast<uintptr_t>(write_pipe_handle.get()));
  std::vector<wchar_t> command_buffer(command.begin(), command.end());
  command_buffer.push_back(L'\0');
  STARTUPINFOW startup{};
  startup.cb = sizeof(startup);
  PROCESS_INFORMATION process_info{};
  BOOL created = CreateProcessAsUserW(restricted.get(), path.c_str(), command_buffer.data(), nullptr,
                                      nullptr, TRUE, CREATE_NO_WINDOW, nullptr, nullptr, &startup,
                                      &process_info);
  DWORD create_error = created ? ERROR_SUCCESS : GetLastError();
  write_pipe_handle.reset();

  DWORD wait_result = WAIT_FAILED;
  DWORD child_exit = STILL_ACTIVE;
  ScopedHandle child_process(created ? process_info.hProcess : nullptr);
  ScopedHandle child_thread(created ? process_info.hThread : nullptr);
  if (created) {
    wait_result = WaitForSingleObject(child_process.get(), kWaitMilliseconds);
    if (wait_result == WAIT_TIMEOUT) {
      TerminateProcess(child_process.get(), 26);
      WaitForSingleObject(child_process.get(), 1000);
    }
    GetExitCodeProcess(child_process.get(), &child_exit);
  }

  std::string child_output;
  char buffer[4096];
  for (;;) {
    DWORD received = 0;
    if (!ReadFile(read_pipe.get(), buffer, sizeof(buffer), &received, nullptr) || received == 0) break;
    child_output.append(buffer, buffer + received);
  }

  std::ofstream output{std::filesystem::path(output_path), std::ios::binary | std::ios::trunc};
  output << "launcher_pid=" << GetCurrentProcessId() << "\n";
  output << "launcher_sid=" << narrow(sid) << "\n";
  output << "launcher_integrity=" << parent_integrity << "\n";
  output << "create_process=" << (created ? "ok" : "failed") << "\n";
  output << "create_process_error=" << create_error << "\n";
  if (created) output << "wait_result=" << wait_result << "\n";
  if (created) output << "child_exit=" << child_exit << "\n";
  output << child_output;
  if (!created) return 28;
  if (wait_result != WAIT_OBJECT_0) return 27;
  return static_cast<int>(child_exit);
}

}  // namespace

int wmain(int argc, wchar_t** argv) {
  if (argc == 5 && std::wstring(argv[1]) == L"--probe") {
    return probe(argv[2], argv[3], static_cast<uintptr_t>(_wcstoui64(argv[4], nullptr, 10)));
  }
  if (argc == 4 && std::wstring(argv[1]) == L"--launch-low") {
    return launch_low(argv[2], argv[3]);
  }
  return 2;
}
