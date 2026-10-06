#pragma once
#include <string>
// Rebuild argv with Profile locale before URLs, preserving Windows quoting.
int EnvBoxEnsureChromiumLocale(std::wstring *command, const wchar_t *locale);
std::wstring EnvBoxCommandImage(const wchar_t *command);
std::wstring EnvBoxQuoteWindowsArgument(const std::wstring &argument);
