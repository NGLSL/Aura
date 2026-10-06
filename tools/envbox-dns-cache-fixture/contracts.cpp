#include "../../runtime/src/dns_response_cache.h"
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <thread>
#include <vector>

using Packet = std::vector<unsigned char>;
void Check(bool value, const char* label) { if (!value) { fprintf(stderr, "FAIL: %s\n", label); std::exit(1); } }
void U16(Packet& p, unsigned value) { p.push_back(static_cast<unsigned char>(value >> 8)); p.push_back(static_cast<unsigned char>(value)); }
void U32(Packet& p, unsigned value) { U16(p, value >> 16); U16(p, value); }
unsigned Read32(const Packet& p, size_t at) { return (unsigned(p[at]) << 24) | (unsigned(p[at + 1]) << 16) | (unsigned(p[at + 2]) << 8) | p[at + 3]; }
Packet Header(unsigned answers = 1, unsigned additional = 0) {
    Packet p{0x12, 0x34, 0x81, 0x80}; U16(p, 1); U16(p, answers); U16(p, 0); U16(p, additional);
    p.insert(p.end(), {1, 'x', 0}); U16(p, 1); U16(p, 1); return p;
}
size_t RR(Packet& p, unsigned type, unsigned ttl, const Packet& data, unsigned klass = 1) {
    p.insert(p.end(), {0xc0, 12}); U16(p, type); U16(p, klass);
    size_t offset = p.size(); U32(p, ttl); U16(p, static_cast<unsigned>(data.size()));
    p.insert(p.end(), data.begin(), data.end()); return offset;
}
Packet Address(unsigned ttl = 30) { Packet p = Header(); RR(p, 1, ttl, {10, 99, 0, 1}); return p; }
bool Store(const std::string& k, const Packet& p, ULONGLONG now = 1000) {
    return EnvBoxDnsResponseCache::Store(k, p.data(), static_cast<int>(p.size()), now);
}
int Lookup(const std::string& k, Packet& out, ULONGLONG now = 1000) {
    return EnvBoxDnsResponseCache::Lookup(k, out.data(), static_cast<int>(out.size()), now);
}
int main() {
    Packet out(65535);
    auto original = Address();
    Check(Store("udp:a/1/0", original), "positive store");
    original.back() = 9;
    Check(Lookup("udp:a/1/0", out) > 0 && out[34] == 1, "store owns bytes");
    out[34] = 8;
    Check(Lookup("udp:a/1/0", out, 2999) > 0 && out[34] == 1 && Read32(out, 25) == 29, "hit owns bytes and ages TTL");
    Check(Lookup("udp:a/1/0", out, 31000) == 0, "TTL boundary expires");
    Check(!Store("zero", Address(0)), "zero TTL not cached");
    Check(Store("cap", Address(3600)) && Lookup("cap", out, 60999) > 0 && Lookup("cap", out, 61000) == 0, "60 second cap");
    Check(Store("clock", Address()) && Lookup("clock", out, 999) == 0, "clock reversal misses");

    Packet chain = Header(2, 1);
    size_t cname = RR(chain, 5, 20, {0xc0, 12});
    size_t address = RR(chain, 1, 10, {10, 99, 0, 1});
    chain.push_back(0); U16(chain, 41); U16(chain, 1232);
    size_t opt = chain.size(); U32(chain, 0x00008000); U16(chain, 0);
    Check(Store("chain", chain) && Lookup("chain", out, 3500) > 0, "compressed chain and OPT");
    Check(Read32(out, cname) == 18 && Read32(out, address) == 8 && Read32(out, opt) == 0x8000, "all normal TTLs aged, OPT unchanged");
    Check(Lookup("chain", out, 11000) == 0, "minimum chain TTL expires");
    Packet authority = Header(); authority[9] = 1;
    RR(authority, 1, 30, {10, 0, 0, 1}); RR(authority, 2, 2, {0xc0, 12});
    Check(Store("authority", authority) && Lookup("authority", out, 3000) == 0, "authority TTL limits lifetime");
    Packet additional = Header(1, 1);
    RR(additional, 1, 30, {10, 0, 0, 1}); RR(additional, 1, 1, {10, 0, 0, 2});
    Check(Store("additional", additional) && Lookup("additional", out, 2000) == 0, "additional TTL limits lifetime");

    auto bad = Address(); bad[2] |= 2; Check(!Store("bad", bad), "truncated response rejected");
    bad = Address(); bad[3] |= 3; Check(!Store("bad", bad), "negative rejected");
    bad = Header(0); Check(!Store("bad", bad), "empty answer rejected");
    bad = Address(); bad[2] &= 0x7f; Check(!Store("bad", bad), "query rejected");
    bad = Address(); bad[2] |= 8; Check(!Store("bad", bad), "nonstandard opcode rejected");
    bad = Address(); bad.pop_back(); Check(!Store("bad", bad), "short RDATA rejected");
    bad = Address(); bad.push_back(0); Check(!Store("bad", bad), "trailing bytes rejected");
    bad = Address(); bad[19] = 0xff; bad[20] = 0xff; Check(!Store("bad", bad), "compression out of bounds rejected");
    bad = Address(); bad[20] = 19; Check(!Store("bad", bad), "compression self loop rejected");
    bad = Address(); bad[20] = 0; Check(!Store("bad", bad), "compression into header rejected");
    bad = Address(); bad[12] = 64; Check(!Store("bad", bad), "reserved name encoding rejected");
    bad = Address(); bad[7] = 255; Check(!Store("bad", bad), "excess RR count rejected");
    bad = Address(); bad[5] = 2; Check(!Store("bad", bad), "multiple questions rejected");
    bad = Address(); bad[24] = 3; Check(!Store("bad", bad), "non-IN class rejected");
    bad = chain; bad[opt] = 1; Check(!Store("bad", bad), "extended negative RCODE rejected");
    bad = chain; bad[opt + 5] = 1; bad.push_back(0); Check(!Store("bad", bad), "short OPT option rejected");
    bad = Header(); RR(bad, 5, 30, {0xc0, 0xff}); Check(!Store("bad", bad), "malformed CNAME rejected");
    bad = Header(); RR(bad, 16, 30, {5, 'a'}); Check(!Store("bad", bad), "short TXT rejected");
    bad = Header(); RR(bad, 1, 30, {1, 2, 3}); Check(!Store("bad", bad), "invalid A size rejected");
    Check(!Store(std::string(4097, 'x'), Address()), "key bound");
    Check(!EnvBoxDnsResponseCache::Store("invalid", nullptr, 12, 1000) &&
          !EnvBoxDnsResponseCache::Store("invalid", out.data(), 65536, 1000), "input bounds");
    Check(EnvBoxDnsResponseCache::Lookup("missing", nullptr, 1, 1000) == 0, "invalid output misses");

    EnvBoxDnsResponseCache::Reset();
    const std::vector<std::string> keys{"udp:a/name/1/0", "udp:b/name/1/0", "tcp:a/name/1/0", "udp:a/name/28/0", "udp:a/name/1/8"};
    for (size_t i = 0; i < keys.size(); ++i) { auto p = Address(); p.back() = static_cast<unsigned char>(i); Check(Store(keys[i], p), "isolated store"); }
    for (size_t i = 0; i < keys.size(); ++i) Check(Lookup(keys[i], out) > 0 && out[34] == i, "endpoint type options isolation");
    Packet tiny(1); Check(Lookup(keys[0], tiny) == 0 && Lookup(keys[0], out) > 0, "small output does not evict");
    EnvBoxDnsResponseCache::Reset();
    for (int i = 0; i < 64; ++i) Check(Store(std::to_string(i), Address()), "fill table");
    Check(Lookup("0", out) > 0 && Store("64", Address()), "LRU insertion");
    Check(Lookup("0", out) > 0 && Lookup("1", out) == 0, "least recently used evicted");
    EnvBoxDnsResponseCache::Reset();
    Packet large = Header(); RR(large, 65280, 30, Packet(60000, 0x5a));
    for (int i = 0; i < 24; ++i) Check(Store(std::to_string(i), large), "large store");
    int retained = 0; for (int i = 0; i < 24; ++i) if (Lookup(std::to_string(i), out)) ++retained;
    Check(retained <= 17 && retained >= 15 && Lookup("23", out) > 0, "byte budget evicts before entry limit");
    EnvBoxDnsResponseCache::Reset();
    std::atomic<bool> healthy{true}; std::vector<std::thread> threads;
    for (int t = 0; t < 8; ++t) threads.emplace_back([t, &healthy] {
        Packet local(65535), p = Address(); const auto key = "thread" + std::to_string(t);
        for (int i = 0; i < 1000; ++i) {
            if (!Store(key, p) || Lookup(key, local) != static_cast<int>(p.size()) || local[34] != 1) healthy = false;
        }
    });
    for (auto& t : threads) t.join(); Check(healthy, "concurrent stores and lookups");
    EnvBoxDnsResponseCache::Reset(); Check(Store("measure", Address()), "measurement store");
    auto start = std::chrono::steady_clock::now();
    for (int i = 0; i < 100000; ++i) Check(Lookup("measure", out) > 0, "measurement hit");
    auto micros = std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - start).count();
    printf("PASS DNS wire response cache contracts; 100000 local hits: %lld us (not network latency)\n", static_cast<long long>(micros));
}
