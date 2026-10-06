#pragma once

#include <windows.h>
#include <string>

// Process-local positive wire responses. now is a monotonic millisecond clock.
// Keys must include the Profile upstream, question and effective query options.
// Lookup preserves the stored transaction ID; the caller restores its own ID.
namespace EnvBoxDnsResponseCache {
int Lookup(const std::string& key, unsigned char* out, int capacity, ULONGLONG now) noexcept;
bool Store(const std::string& key, const unsigned char* packet, int length, ULONGLONG now) noexcept;
void Reset() noexcept;
}
