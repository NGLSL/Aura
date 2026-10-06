#include <winsock2.h>
#include <ws2tcpip.h>
#include "runtime_profile.h"
#include <cstdio>
#include <cstring>

namespace {
bool Number(const char* text, unsigned maximum, unsigned* value) {
  if (!text || !*text) return false;
  unsigned result = 0;
  for (const char* p = text; *p; ++p) {
    if (*p < '0' || *p > '9') return false;
    unsigned digit = static_cast<unsigned>(*p - '0');
    if (digit > maximum || result > (maximum - digit) / 10) return false;
    result = result * 10 + digit;
  }
  if (result > maximum) return false;
  *value = result;
  return true;
}
bool Literal(const char* text) {
  in_addr v4;
  in6_addr v6;
  return InetPtonA(AF_INET, text, &v4) == 1 || InetPtonA(AF_INET6, text, &v6) == 1;
}
bool Required(EnvBoxDnsFieldGetter getter, void* context, const char* key,
              char* value, size_t capacity) {
  return getter(context, key, value, capacity) == 1;
}
bool GetNumber(EnvBoxDnsFieldGetter getter, void* context, const char* key,
                unsigned maximum, unsigned* value) {
  char text[16];
  return Required(getter, context, key, text, sizeof(text)) && Number(text, maximum, value);
}
void AddViewIp(RuntimeProfile* out, const char* ip) {
  for (int i = 0; i < out->dns_server_count; ++i)
    if (_stricmp(out->dns_servers[i], ip) == 0) return;
  if (out->dns_server_count < ENVBOX_DNS_MAX)
    strcpy_s(out->dns_servers[out->dns_server_count++], ip);
}
}

int EnvBoxDecodeDnsConfiguration(RuntimeProfile* out, EnvBoxDnsFieldGetter getter,
                                 void* context) {
  if (!out || !getter) return 0;
  out->dns_config_version = 0;
  out->dns_strict = 1;
  out->dns_upstream_count = 0;
  out->dns_server_count = 0;
  char version[16] = {};
  int has_version = getter(context, "dns_config_version", version, sizeof(version));
  if (has_version < 0) return 0;
  char mode[32] = {};
  int has_mode = getter(context, "dns_mode", mode, sizeof(mode));
  if (has_mode < 0) return 0;
  if (!has_mode || strcmp(mode, "0") == 0 || strcmp(mode, "host") == 0) out->dns_mode = 0;
  else if (strcmp(mode, "1") == 0 || strcmp(mode, "virtual_view") == 0) out->dns_mode = 1;
  else return 0;
  if (!has_version) {
    char legacy[ENVBOX_DNS_MAX * 64] = {};
    int found = getter(context, "dns_servers", legacy, sizeof(legacy));
    if (found < 0) return 0;
    if (!found || !legacy[0]) return 1;
    char* next = legacy;
    for (;;) {
      char* separator = strchr(next, ';');
      if (separator) *separator = '\0';
      if (!Literal(next) || out->dns_server_count == ENVBOX_DNS_MAX || strlen(next) >= 64) return 0;
      strcpy_s(out->dns_servers[out->dns_server_count++], next);
      if (!separator) break;
      next = separator + 1;
    }
    return 1;
  }
  unsigned numeric = 0;
  if (!Number(version, 1, &numeric) || numeric != 1 || !has_mode) return 0;
  out->dns_config_version = 1;
  char strict[8] = {};
  if (!Required(getter, context, "dns_strict", strict, sizeof(strict))) return 0;
  if (strcmp(strict, "1") == 0 || strcmp(strict, "true") == 0) out->dns_strict = 1;
  else if (strcmp(strict, "0") == 0 || strcmp(strict, "false") == 0) out->dns_strict = 0;
  else return 0;
  if (out->dns_mode == 1 && out->dns_strict != 1) return 0;
  if (!GetNumber(getter, context, "dns_upstream_count", ENVBOX_DNS_MAX, &numeric)) return 0;
  out->dns_upstream_count = static_cast<int>(numeric);
  for (int i = 0; i < out->dns_upstream_count; ++i) {
    RuntimeDnsUpstream& entry = out->dns_upstreams[i];
    entry = {};
    char key[96], type[8];
    auto field = [&](const char* name, char* value, size_t capacity) {
      sprintf_s(key, "dns_upstream_%d_%s", i, name);
      return Required(getter, context, key, value, capacity);
    };
    if (!field("type", type, sizeof(type))) return 0;
    if (strcmp(type, "udp") == 0) entry.type = EnvBoxDnsUdp;
    else if (strcmp(type, "tcp") == 0) entry.type = EnvBoxDnsTcp;
    else if (strcmp(type, "dot") == 0) entry.type = EnvBoxDnsDot;
    else if (strcmp(type, "doh") == 0) entry.type = EnvBoxDnsDoh;
    else return 0;
    if (entry.type != EnvBoxDnsDoh) {
      char port[16];
      if (!field("address", entry.address, sizeof(entry.address)) || !Literal(entry.address) ||
          !field("port", port, sizeof(port)) || !Number(port, 65535, &numeric) || numeric == 0) return 0;
      entry.port = static_cast<unsigned short>(numeric);
      AddViewIp(out, entry.address);
      if (entry.type == EnvBoxDnsDot &&
          (!field("server_name", entry.server_name, sizeof(entry.server_name)) || !entry.server_name[0])) return 0;
    } else {
      char count[16], revocation[8];
      if (!field("url", entry.url, sizeof(entry.url)) || strncmp(entry.url, "https://", 8) != 0 ||
          !entry.url[8] || strchr(entry.url, '\r') || strchr(entry.url, '\n') ||
          !field("bootstrap_count", count, sizeof(count)) || !Number(count, ENVBOX_DNS_MAX, &numeric)) return 0;
      entry.bootstrap_count = static_cast<int>(numeric);
      if (!field("tls_revocation", revocation, sizeof(revocation)) ||
          !Number(revocation, 1, &numeric)) return 0;
      entry.tls_revocation = static_cast<int>(numeric);
      for (int b = 0; b < entry.bootstrap_count; ++b) {
        sprintf_s(key, "dns_upstream_%d_bootstrap_%d", i, b);
        if (!Required(getter, context, key, entry.bootstrap_ips[b], sizeof(entry.bootstrap_ips[b])) ||
            !Literal(entry.bootstrap_ips[b])) return 0;
        AddViewIp(out, entry.bootstrap_ips[b]);
      }
    }
  }
  return 1;
}

int EnvBoxEmitDnsConfiguration(const RuntimeProfile* p,
                               EnvBoxDnsFieldSetter setter, void* context) {
  if (!p || !setter) return 0;
  if (!setter(context, "dns_mode", p->dns_mode ? "virtual_view" : "host")) return 0;
  if (p->dns_config_version == 0) {
    char legacy[ENVBOX_DNS_MAX * 64] = {};
    if (p->dns_server_count < 0 || p->dns_server_count > ENVBOX_DNS_MAX) return 0;
    for (int i = 0; i < p->dns_server_count; ++i) {
      if (i) strcat_s(legacy, ";");
      strcat_s(legacy, p->dns_servers[i]);
    }
    return setter(context, "dns_servers", legacy);
  }
  if (p->dns_config_version != 1 || p->dns_upstream_count < 0 ||
      p->dns_upstream_count > ENVBOX_DNS_MAX ||
      !setter(context, "dns_config_version", "1") ||
      !setter(context, "dns_strict", p->dns_strict ? "1" : "0")) return 0;
  char number[16];
  sprintf_s(number, "%d", p->dns_upstream_count);
  if (!setter(context, "dns_upstream_count", number)) return 0;
  for (int i = 0; i < p->dns_upstream_count; ++i) {
    const RuntimeDnsUpstream& entry = p->dns_upstreams[i];
    char key[96];
    auto field = [&](const char* name, const char* value) {
      sprintf_s(key, "dns_upstream_%d_%s", i, name);
      return setter(context, key, value);
    };
    const char* type = entry.type == EnvBoxDnsUdp ? "udp" : entry.type == EnvBoxDnsTcp ? "tcp" :
                       entry.type == EnvBoxDnsDot ? "dot" : entry.type == EnvBoxDnsDoh ? "doh" : nullptr;
    if (!type || !field("type", type)) return 0;
    if (entry.type != EnvBoxDnsDoh) {
      sprintf_s(number, "%u", entry.port);
      if (!field("address", entry.address) || !field("port", number)) return 0;
      if (entry.type == EnvBoxDnsDot && !field("server_name", entry.server_name)) return 0;
    } else {
      if (entry.tls_revocation < 0 || entry.tls_revocation > 1 ||
          !field("tls_revocation", entry.tls_revocation ? "1" : "0")) return 0;
      sprintf_s(number, "%d", entry.bootstrap_count);
      if (entry.bootstrap_count < 0 || entry.bootstrap_count > ENVBOX_DNS_MAX ||
          !field("url", entry.url) || !field("bootstrap_count", number)) return 0;
      for (int b = 0; b < entry.bootstrap_count; ++b) {
        sprintf_s(key, "dns_upstream_%d_bootstrap_%d", i, b);
        if (!setter(context, key, entry.bootstrap_ips[b])) return 0;
      }
    }
  }
  return 1;
}
