#include "browser_child_locale.h"
#include <windows.h>
#include <shellapi.h>
#include <vector>
std::wstring EnvBoxCommandImage(const wchar_t *command) {
  if (!command || !*command) return {};
  int count = 0;
  LPWSTR *args = CommandLineToArgvW(command, &count);
  if (!args) return {};
  std::wstring image = count ? args[0] : L"";
  LocalFree(args);
  return image;
}
std::wstring EnvBoxQuoteWindowsArgument(const std::wstring &arg) {
  if (!arg.empty() && arg.find_first_of(L" \t\n\v\"") == std::wstring::npos)
    return arg;
  std::wstring out = L"\"";
  size_t slashes = 0;
  for (wchar_t c : arg) {
    if (c == L'\\') {
      ++slashes;
      continue;
    }
    out.append(c == L'"' ? slashes * 2 + 1 : slashes, L'\\');
    out.push_back(c);
    slashes = 0;
  }
  out.append(slashes * 2, L'\\');
  out.push_back(L'"');
  return out;
}
int EnvBoxEnsureChromiumLocale(std::wstring *command, const wchar_t *locale) {
  if (!command || command->empty() || !locale || !*locale) return 0;
  int count = 0;
  LPWSTR *parsed = CommandLineToArgvW(command->c_str(), &count);
  if (!parsed || !count) {
    if (parsed) LocalFree(parsed);
    return 0;
  }
  std::vector<std::wstring> args;
  args.emplace_back(parsed[0]);
  std::wstring value = locale,
               language = value.substr(0, value.find_first_of(L"-_"));
  args.emplace_back(L"--lang=" + value);
  args.emplace_back(
      L"--accept-lang=" + value +
      (_wcsicmp(value.c_str(), language.c_str()) ? L"," + language : L""));
  for (int i = 1; i < count; ++i) {
    std::wstring arg = parsed[i];
    auto eq = arg.find(L'=');
    std::wstring key = arg.substr(0, eq);
    if (!_wcsicmp(key.c_str(), L"--lang") ||
        !_wcsicmp(key.c_str(), L"--accept-lang")) {
      if (eq == std::wstring::npos && i + 1 < count && parsed[i + 1][0] != L'-')
        ++i;
      continue;
    }
    args.push_back(arg);
  }
  LocalFree(parsed);
  std::wstring out;
  for (const auto &arg : args) {
    if (!out.empty()) out += L' ';
    out += EnvBoxQuoteWindowsArgument(arg);
  }
  if (out == *command) return 0;
  *command = out;
  return 1;
}
