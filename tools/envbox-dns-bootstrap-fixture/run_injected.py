"""Actual Runtime injection with owned UDP/TCP bootstrap listeners and public DoH.

The local listener answers only the DoH authority's A/CNAME questions. It never
answers the application name, so success cannot be a fallback to the seed.
No Host DNS/trust/configuration changes. Public TLS is temporal service evidence.
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
import uuid


def name(value):
    return b"".join(bytes([len(part)]) + part.encode("ascii") for part in value.split(".")) + b"\0"


class Seed:
    def __init__(self, transport):
        self.transport = transport
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if transport == "udp" else socket.SOCK_STREAM)
        self.socket.bind(("127.0.0.1", 0))
        self.port = self.socket.getsockname()[1]
        self.socket.settimeout(0.1)
        if transport == "tcp":
            self.socket.listen(8)
        self.wire = []
        self.errors = []
        self.stop = threading.Event()
        self.worker = threading.Thread(target=self.run)
        self.worker.start()

    def answer(self, query):
        offset = 12
        labels = []
        while query[offset]:
            length = query[offset]
            labels.append(query[offset + 1:offset + length + 1].decode("ascii"))
            offset += length + 1
        offset += 1
        host = ".".join(labels)
        kind = struct.unpack("!H", query[offset:offset + 2])[0]
        self.wire.append({"name": host, "qtype": kind, "packet": query.hex()})
        end = offset + 4
        if kind != 1 or host not in ("cloudflare-dns.com", "alias.bootstrap.invalid"):
            return query[:2] + struct.pack("!HHHHH", 0x8182, 1, 0, 0, 0) + query[12:end]
        record_type = 5 if host == "cloudflare-dns.com" else 1
        data = name("alias.bootstrap.invalid") if record_type == 5 else bytes([1, 1, 1, 1])
        return (query[:2] + struct.pack("!HHHHH", 0x8180, 1, 1, 0, 0) + query[12:end]
                + b"\xc0\x0c" + struct.pack("!HHIH", record_type, 1, 1, len(data)) + data)

    def run(self):
        try:
            while not self.stop.is_set():
                try:
                    if self.transport == "udp":
                        query, peer = self.socket.recvfrom(65535)
                        self.socket.sendto(self.answer(query), peer)
                    else:
                        client, _ = self.socket.accept()
                        with client:
                            client.settimeout(2)
                            prefix = self.read(client, 2)
                            query = self.read(client, struct.unpack("!H", prefix)[0])
                            reply = self.answer(query)
                            client.sendall(struct.pack("!H", len(reply)) + reply)
                except socket.timeout:
                    continue
        except Exception as error:
            self.errors.append(repr(error))

    @staticmethod
    def read(client, count):
        data = b""
        while len(data) < count:
            part = client.recv(count - len(data))
            if not part:
                raise RuntimeError("incomplete TCP query")
            data += part
        return data

    def close(self):
        self.stop.set()
        self.worker.join(3)
        self.socket.close()
        if self.worker.is_alive() or self.errors:
            raise AssertionError(self.errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--probe64", required=True)
    parser.add_argument("--probe32", required=True)
    parser.add_argument("--runtime-dir", required=True)
    args = parser.parse_args()
    kernel = ctypes.WinDLL("kernel32")
    kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
    kernel.GetModuleHandleW.restype = ctypes.c_void_p
    assert not any(kernel.GetModuleHandleW(f"envbox-runtime{arch}.dll") for arch in ("64", "32")), "Controller is injected"
    root = pathlib.Path(__file__).resolve().parents[2] / "target" / ("dns-profile-bootstrap-" + uuid.uuid4().hex)
    root.mkdir()
    actors = {"cli": pathlib.Path(args.cli).resolve(), "probe64": pathlib.Path(args.probe64).resolve(),
              "probe32": pathlib.Path(args.probe32).resolve()}
    actors.update({"runtime" + arch: pathlib.Path(args.runtime_dir).resolve() / ("envbox-runtime" + arch + ".dll") for arch in ("64", "32")})
    evidence = {"controller_pid": os.getpid(), "controller_runtime_loaded": False,
                "actors": {key: {"path": str(value), "sha256": hashlib.sha256(value.read_bytes()).hexdigest().upper()} for key, value in actors.items()},
                "rows": [], "pass": False}
    seeds = [Seed("udp"), Seed("tcp")]
    base = {key: value for key, value in os.environ.items() if not key.startswith("ENVBOX_")}

    def command(env, *values):
        result = subprocess.run([str(actors["cli"]), *values], env=env, capture_output=True, text=True, timeout=30)
        if result.returncode:
            raise AssertionError((values, result.returncode, result.stdout, result.stderr))
        return result.stdout.strip()

    def profile(env, label, seed):
        identifier = command(env, "profile", "add", "--name", label, "--locale", "en-US", "--ui-language", "en-US", "--region", "US",
                             "--tz-windows", "Pacific Standard Time", "--tz-iana", "America/Los_Angeles", "--dns-mode", "host")
        command(env, "profile", "dns", "add", identifier, "--type", "doh", "--url", "https://cloudflare-dns.com/dns-query")
        if seed:
            command(env, "profile", "dns", "add", identifier, "--type", seed.transport, "--address", "127.0.0.1", "--port", str(seed.port))
        command(env, "profile", "dns", "set", identifier, "--mode", "virtual_view", "--strict", "true")
        return identifier

    try:
        for arch in ("64", "32"):
            env = dict(base, ENVBOX_CONFIG_ROOT=str(root / ("config" + arch)), ENVBOX_RUNTIME_DLL=str(actors["runtime" + arch]))
            for seed in seeds + [None]:
                label = (seed.transport if seed else "no-seed") + arch
                identifier = profile(env, label, seed)
                for route in (("qtype65", "getaddrinfo") if seed and seed.transport == "udp" else ("qtype65",)):
                    before = len(seed.wire) if seed else 0
                    query = ("--resolve", "example.com") if route == "getaddrinfo" else ("--dns-rr", "cloudflare.com", "65", "w")
                    text = command(env, "run", "--profile", identifier, "--audit", str(actors["probe" + arch]), *query)
                    wire = seed.wire[before:] if seed else []
                    valid = ("getaddrinfo:\n<error" not in text and "getaddrinfo:\n<empty>" not in text) if route == "getaddrinfo" else (
                        "DnsRR_Status:\n0\n" in text or (seed is not None and "DnsRR_Status:\n9501\n" in text))
                    if seed is None:
                        valid = "DnsRR_Status:\n1460\n" in text
                    row = {"arch": arch, "profile": label, "route": route, "stdout": text, "request_wire": wire,
                           "pass": valid and "EnvBox Runtime Loaded" in text and (not seed or len(wire) >= 2 and
                               all(item["qtype"] == 1 and item["name"] in ("cloudflare-dns.com", "alias.bootstrap.invalid") for item in wire))}
                    evidence["rows"].append(row)
                    print(json.dumps({key: row[key] for key in ("arch", "profile", "route", "pass")}), flush=True)
        audit = "\n".join(path.read_text(encoding="utf-8") for path in root.glob("config*/audit/*.jsonl"))
        evidence["host_fallback_audit_entries"] = audit.count('"dns-host"')
        evidence["pass"] = all(row["pass"] for row in evidence["rows"]) and not evidence["host_fallback_audit_entries"]
    finally:
        for seed in seeds:
            seed.close()
        (root / "result.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
        print(json.dumps({"evidence": str(root / "result.json"), "pass": evidence["pass"]}), flush=True)
    if not evidence["pass"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
