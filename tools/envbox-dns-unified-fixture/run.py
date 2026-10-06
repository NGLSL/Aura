"""IPv4 actual-injection DNS matrix. No host network or trust changes.

Public TLS answers are service evidence, not deterministic record fixtures.
Every invocation preserves a unique target evidence/configuration directory.
"""
import argparse
import ctypes
import hashlib
import json
import os
import pathlib
import socket
import struct
import subprocess
import threading
import time
import uuid


APIS = ("a", "w", "utf8", "ex", "async")
CASES = [("rr.fixture.test", kind) for kind in (1, 28, 65, 64, 16, 12, 33, 5, 2, 65280)] + [(".", 2)]


def field(text, key):
    marker = key + ":\n"
    return text.split(marker, 1)[1].splitlines()[0] if marker in text else None


def actor(path):
    path = pathlib.Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest().upper()}


def encode_name(name):
    return b"".join(bytes([len(part)]) + part.encode("ascii") for part in name.split(".")) + b"\0"


def answer(query):
    offset = 12
    while query[offset]:
        offset += query[offset] + 1
    offset += 1
    kind = struct.unpack("!H", query[offset:offset + 2])[0]
    end = offset + 4
    data = {1: bytes([10, 99, 0, 1]), 28: bytes.fromhex("20010db8000000000000000000000001"),
            64: b"\0\1\0", 65: b"\0\1\0", 16: b"\x0eprofile-marker",
            2: encode_name("target.fixture.test"), 5: encode_name("target.fixture.test"),
            12: encode_name("target.fixture.test"),
            33: struct.pack("!HHH", 7, 11, 443) + encode_name("target.fixture.test"),
            65280: bytes.fromhex("deadbeef")}[kind]
    return (query[:2] + struct.pack("!HHHHH", 0x8480, 1, 1, 0, 0) + query[12:end]
            + b"\xc0\x0c" + struct.pack("!HHIH", kind, 1, 60, len(data)) + data)


class Server:
    def __init__(self, transport, silent=False):
        self.transport, self.silent = transport, silent
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if transport == "udp" else socket.SOCK_STREAM)
        self.socket.bind(("127.0.0.1", 0))
        self.port = self.socket.getsockname()[1]
        self.socket.settimeout(.1)
        if transport == "tcp":
            self.socket.listen(32)
        self.stop = threading.Event()
        self.wire = []
        self.errors = []
        self.worker = threading.Thread(target=self.run)
        self.worker.start()

    def run(self):
        while not self.stop.is_set():
            try:
                if self.transport == "udp":
                    query, peer = self.socket.recvfrom(65535)
                    self.wire.append(query.hex())
                    if not self.silent:
                        self.socket.sendto(answer(query), peer)
                else:
                    client, _ = self.socket.accept()
                    with client:
                        client.settimeout(1)
                        length = struct.unpack("!H", self.exact(client, 2))[0]
                        query = self.exact(client, length)
                        self.wire.append(query.hex())
                        if self.silent:
                            client.settimeout(.1)
                            while not self.stop.is_set():
                                try:
                                    if not client.recv(1):
                                        break
                                except socket.timeout:
                                    continue
                        else:
                            response = answer(query)
                            client.sendall(struct.pack("!H", len(response)) + response)
            except socket.timeout:
                continue
            except (EOFError, ConnectionError):
                continue
            except Exception as error:
                self.errors.append(repr(error))

    @staticmethod
    def exact(client, count):
        value = b""
        while len(value) < count:
            block = client.recv(count - len(value))
            if not block:
                raise EOFError()
            value += block
        return value

    def close(self):
        self.stop.set()
        self.worker.join(3)
        self.socket.close()
        if self.worker.is_alive() or self.errors:
            raise AssertionError(("fixture worker", self.errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--probe64", required=True)
    parser.add_argument("--probe32", required=True)
    parser.add_argument("--runtime-dir", required=True)
    parser.add_argument("--local-only", action="store_true")
    args = parser.parse_args()
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
    kernel.GetModuleHandleW.restype = ctypes.c_void_p

    def clean():
        loaded = [name for name in ("envbox-runtime64.dll", "envbox-runtime32.dll") if kernel.GetModuleHandleW(name)]
        if loaded:
            raise AssertionError(("injected controller", loaded))
        return loaded

    clean()
    repo = pathlib.Path(__file__).resolve().parents[2]
    root = repo / "target" / ("dns-unified-" + uuid.uuid4().hex)
    root.mkdir()
    actors = {"cli": actor(args.cli), "probe64": actor(args.probe64), "probe32": actor(args.probe32)}
    actors.update({"runtime" + arch: actor(pathlib.Path(args.runtime_dir) / ("envbox-runtime" + arch + ".dll")) for arch in ("64", "32")})
    evidence = {"controller_pid": os.getpid(), "controller_runtime_before": clean(), "actors": actors,
                "ipv6": "deferred", "config_root": str(root), "rows": [], "pass": False,
                "boundaries": ["Audit is not independent global traffic capture", "No GUI exit, parent/child or resource soak coverage"]}
    servers = [Server("udp"), Server("tcp"), Server("tcp", silent=True)]
    base = {key: value for key, value in os.environ.items() if not key.startswith("ENVBOX_")}
    profile_names = {}

    def command(env, *values):
        clean()
        result = subprocess.run([actors["cli"]["path"], *values], env=env, capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise AssertionError((values, result.returncode, result.stdout, result.stderr))
        return result.stdout.strip()

    def profile(env, label, upstreams):
        identifier = command(env, "profile", "add", "--name", label, "--locale", "en-US", "--ui-language", "en-US",
                             "--region", "US", "--tz-windows", "Pacific Standard Time", "--tz-iana", "America/Los_Angeles", "--dns-mode", "host")
        for upstream in upstreams:
            command(env, "profile", "dns", "add", identifier, *upstream)
        command(env, "profile", "dns", "set", identifier, "--mode", "virtual_view", "--strict", "true")
        profile_names[identifier] = label
        return identifier

    def query(env, arch, identifier, transport, name, kind, api, expected, cancel=False, server=None):
        before = len(server.wire) if server else None
        values = ["run", "--profile", identifier, "--audit", actors["probe" + arch]["path"], "--dns-rr", name, str(kind), api]
        if cancel:
            values += ["264", "--cancel"]
        start = time.monotonic()
        text = command(env, *values)
        row = {"architecture": arch, "transport": transport, "name": name, "qtype": kind, "api": api,
               "profile_id": identifier, "profile_name": profile_names[identifier], "expected_status": sorted(expected),
               "status": field(text, "DnsRR_Status"), "freed": field(text, "DnsRR_Freed"),
               "records": field(text, "DnsRR_Records"), "seconds": time.monotonic() - start,
               "cancel_status": field(text, "DnsRR_CancelStatus"), "stdout": text,
               "request_wire": server.wire[before:] if server else None}
        row["pass"] = (row["status"] in expected and row["freed"] == "true" and "EnvBox Runtime Loaded" in text
                       and (not server or len(row["request_wire"]) == 1)
                       and (not cancel or row["cancel_status"] == "0"))
        if server and not server.silent:
            row["pass"] = row["pass"] and row["records"] == "1" and f"type={kind} " in text
        evidence["rows"].append(row)
        print(json.dumps({key: row[key] for key in ("architecture", "transport", "qtype", "api", "status", "pass")}), flush=True)
        (root / "result.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")

    try:
        for arch in ("64", "32"):
            env = dict(base, ENVBOX_CONFIG_ROOT=str(root / ("config" + arch)), ENVBOX_RUNTIME_DLL=actors["runtime" + arch]["path"])
            for server in servers[:2]:
                ident = profile(env, server.transport + arch, [["--type", server.transport, "--address", "127.0.0.1", "--port", str(server.port)]])
                for name, kind in CASES:
                    for api in APIS:
                        query(env, arch, ident, server.transport, name, kind, api, {"0"}, server=server)
            ordered = profile(env, "ordered" + arch, [["--type", "tcp", "--address", "127.0.0.1", "--port", "1"],
                                                     ["--type", "udp", "--address", "127.0.0.1", "--port", str(servers[0].port)]])
            for api in APIS:
                query(env, arch, ordered, "tcp-fail-udp-success", "rr.fixture.test", 16, api, {"0"}, server=servers[0])
            dead = profile(env, "dead" + arch, [["--type", "tcp", "--address", "127.0.0.1", "--port", "1"]])
            for api in APIS:
                query(env, arch, dead, "all-failed", "rr.fixture.test", 65, api, {"1460"})
            stalled = profile(env, "stalled" + arch, [["--type", "tcp", "--address", "127.0.0.1", "--port", str(servers[2].port)]])
            query(env, arch, stalled, "tcp-cancel", "rr.fixture.test", 65, "async", {"1223"}, cancel=True, server=servers[2])
            query(env, arch, stalled, "tcp-deadline", "rr.fixture.test", 65, "w", {"1460"}, server=servers[2])
            if not args.local_only:
                for transport, upstream in (("dot", ["--type", "dot", "--address", "1.1.1.1", "--port", "853", "--server-name", "cloudflare-dns.com"]),
                                           ("doh", ["--type", "doh", "--url", "https://cloudflare-dns.com/dns-query", "--bootstrap", "1.1.1.1", "--tls-revocation", "standard"])):
                    ident = profile(env, transport + arch, [upstream])
                    for fixture_name, kind in CASES:
                        name = "." if fixture_name == "." else "cloudflare-dns.com"
                        for api in APIS:
                            query(env, arch, ident, transport, name, kind, api, {"0", "9501", "9003"})
        audit = "\n".join(path.read_text(encoding="utf-8") for path in root.glob("config*/audit/*.jsonl"))
        evidence["audit_host_fallback_entries"] = audit.count('"dns-host"')
        evidence["controller_runtime_after"] = clean()
        evidence["pass"] = all(row["pass"] for row in evidence["rows"]) and evidence["audit_host_fallback_entries"] == 0
    finally:
        for server in servers:
            server.close()
        evidence["summary"] = {"total": len(evidence["rows"]), "passed": sum(row["pass"] for row in evidence["rows"]),
                               "failed": sum(not row["pass"] for row in evidence["rows"])}
        (root / "result.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
        print(json.dumps({"evidence": str(root / "result.json"), "pass": evidence["pass"], **evidence["summary"]}), flush=True)
    if not evidence["pass"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
