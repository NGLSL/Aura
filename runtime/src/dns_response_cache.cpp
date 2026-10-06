#include "dns_response_cache.h"

#include <array>
#include <cstdint>
#include <utility>
#include <vector>

namespace {
constexpr size_t kEntries = 64;
constexpr size_t kBudget = 1024 * 1024 - 16384; // Reserve the fixed table and lock overhead.
constexpr size_t kMaxKey = 4096;
constexpr uint32_t kMaxSeconds = 60;
struct Entry {
    std::string key;
    std::vector<unsigned char> packet;
    std::vector<size_t> ttls;
    ULONGLONG stored = 0;
    ULONGLONG used = 0;
    uint32_t lifetime = 0;
    size_t Cost() const { return sizeof(Entry) + key.capacity() + packet.capacity() + ttls.capacity() * sizeof(size_t); }
};
static_assert(sizeof(Entry) * kEntries + sizeof(SRWLOCK) + 256 < 16384, "fixed cache overhead reserve");
SRWLOCK lock = SRWLOCK_INIT;
std::array<Entry, kEntries> entries;
size_t bytes = 0;
ULONGLONG sequence = 0;
struct Guard {
    Guard() { AcquireSRWLockExclusive(&lock); }
    ~Guard() { ReleaseSRWLockExclusive(&lock); }
};
uint16_t U16(const unsigned char* p) { return static_cast<uint16_t>((p[0] << 8) | p[1]); }
uint32_t U32(const unsigned char* p) {
    return (static_cast<uint32_t>(p[0]) << 24) | (static_cast<uint32_t>(p[1]) << 16) |
           (static_cast<uint32_t>(p[2]) << 8) | p[3];
}
void Put32(unsigned char* p, uint32_t v) {
    p[0] = static_cast<unsigned char>(v >> 24); p[1] = static_cast<unsigned char>(v >> 16);
    p[2] = static_cast<unsigned char>(v >> 8); p[3] = static_cast<unsigned char>(v);
}
// Validate compression targets as well as the bytes consumed at the original name.
bool Name(const unsigned char* p, size_t n, size_t& cursor) {
    size_t at = cursor, end = cursor, expanded = 0;
    bool jumped = false;
    for (size_t steps = 0; steps < 128; ++steps) {
        if (at >= n) return false;
        unsigned char length = p[at++];
        if ((length & 0xc0) == 0xc0) {
            if (at >= n) return false;
            size_t target = ((length & 0x3f) << 8) | p[at++];
            if (target < 12 || target >= at - 2) return false;
            if (!jumped) end = at;
            jumped = true; at = target;
        } else {
            if ((length & 0xc0) != 0 || length > n - at) return false;
            expanded += length + 1;
            if (expanded > 255) return false;
            at += length;
            if (!jumped) end = at;
            if (length == 0) { cursor = end; return true; }
        }
    }
    return false;
}
bool Scan(Entry& entry) {
    const auto* p = entry.packet.data(); const size_t n = entry.packet.size();
    if (n < 12 || (p[2] & 0x80) == 0 || (p[2] & 0x7a) != 0 || (p[3] & 0x0f) != 0) return false;
    const uint16_t questions = U16(p + 4), answers = U16(p + 6);
    if (questions != 1 || answers == 0) return false;
    size_t at = 12;
    if (!Name(p, n, at) || n - at < 4 || U16(p + at + 2) != 1) return false;
    at += 4;
    uint32_t minimum = kMaxSeconds;
    const uint32_t records = static_cast<uint32_t>(answers) + U16(p + 8) + U16(p + 10);
    bool positive = false;
    bool opt = false;
    for (uint32_t i = 0; i < records; ++i) {
        const size_t owner = at;
        if (!Name(p, n, at) || n - at < 10) return false;
        const uint16_t type = U16(p + at), klass = U16(p + at + 2), size = U16(p + at + 8);
        if (size > n - at - 10) return false;
        if (type == 41) {
            // EDNS extended RCODE occupies the top byte of the OPT TTL field.
            if (opt || i < static_cast<uint32_t>(answers) + U16(p + 8) ||
                p[owner] != 0 || p[at + 4] != 0) return false;
            opt = true;
            size_t option = at + 10, end = option + size;
            while (option < end) {
                if (end - option < 4 || U16(p + option + 2) > end - option - 4) return false;
                option += 4 + U16(p + option + 2);
            }
        } else {
            if (klass != 1) return false;
            const size_t data = at + 10, end = data + size;
            size_t name = data;
            if ((type == 1 && size != 4) || (type == 28 && size != 16)) return false;
            if (type == 2 || type == 5 || type == 12 || type == 39) {
                if (!Name(p, n, name) || name != end) return false;
            } else if (type == 15 || type == 33) {
                const size_t prefix = type == 15 ? 2 : 6;
                if (size < prefix) return false;
                name += prefix;
                if (!Name(p, n, name) || name != end) return false;
            } else if (type == 6) {
                if (!Name(p, n, name) || name > end || !Name(p, n, name) ||
                    name > end || end - name != 20) return false;
            } else if (type == 16) {
                while (name < end) {
                    const size_t text = p[name++];
                    if (text > end - name) return false;
                    name += text;
                }
            }
            const uint32_t ttl = U32(p + at + 4);
            if (ttl == 0) return false;
            if (ttl < minimum) minimum = ttl;
            entry.ttls.push_back(at + 4);
            if (i < answers) positive = true;
        }
        at += 10 + size;
    }
    if (at != n || !positive) return false;
    entry.lifetime = minimum;
    return true;
}
bool Expired(const Entry& e, ULONGLONG now) {
    return now < e.stored || now - e.stored >= static_cast<ULONGLONG>(e.lifetime) * 1000;
}
void Remove(Entry& e) {
    if (!e.packet.empty()) bytes -= e.Cost();
    e = Entry{};
}
}

namespace EnvBoxDnsResponseCache {
int Lookup(const std::string& key, unsigned char* out, int capacity, ULONGLONG now) noexcept {
    if (!out || capacity <= 0 || key.empty() || key.size() > kMaxKey) return 0;
    Guard guard;
    for (auto& e : entries) {
        if (e.packet.empty() || e.key != key) continue;
        if (Expired(e, now)) { Remove(e); return 0; }
        if (e.packet.size() > static_cast<size_t>(capacity)) return 0;
        memcpy(out, e.packet.data(), e.packet.size());
        const uint32_t age = static_cast<uint32_t>((now - e.stored) / 1000);
        for (size_t offset : e.ttls) Put32(out + offset, U32(e.packet.data() + offset) - age);
        e.used = ++sequence;
        return static_cast<int>(e.packet.size());
    }
    return 0;
}
bool Store(const std::string& key, const unsigned char* packet, int length, ULONGLONG now) noexcept {
    if (!packet || length < 12 || length > 65535 || key.empty() || key.size() > kMaxKey) return false;
    try {
        Entry fresh;
        fresh.key = key; fresh.packet.assign(packet, packet + length);
        if (!Scan(fresh) || fresh.Cost() > kBudget) return false;
        fresh.stored = now;
        Guard guard;
        for (auto& e : entries) {
            if (!e.packet.empty() && (e.key == key || Expired(e, now))) Remove(e);
        }
        while (bytes + fresh.Cost() > kBudget) {
            Entry* oldest = nullptr;
            for (auto& e : entries) if (!e.packet.empty() && (!oldest || e.used < oldest->used)) oldest = &e;
            if (!oldest) return false;
            Remove(*oldest);
        }
        Entry* slot = nullptr;
        for (auto& e : entries) if (e.packet.empty()) { slot = &e; break; }
        if (!slot) {
            slot = &entries[0];
            for (auto& e : entries) if (e.used < slot->used) slot = &e;
            Remove(*slot);
        }
        fresh.used = ++sequence; bytes += fresh.Cost(); *slot = std::move(fresh);
        return true;
    } catch (...) { return false; }
}
void Reset() noexcept {
    Guard guard;
    for (auto& e : entries) Remove(e);
    sequence = 0;
}
}
