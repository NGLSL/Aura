"""Actual CLI/Runtime DoT failures and explicit fallback, using no test trust.

Usage: python run_injected.py CLI_EXE PROBE_EXE RUNTIME_DLL
Only the generated Profile/configuration directory and loopback fixtures change.
The untouched Host control runs the same Probe directly. No certificate import.
"""
import json
import os
import pathlib
import socket
import ssl
import struct
import subprocess
import sys
import threading

sys.dont_write_bytecode = True
import run as certs


def field(text, key):
    marker = key + ":\n"
    return text.split(marker, 1)[1].splitlines()[0] if marker in text else None


def main():
    cli, probe, dll = map(lambda value: str(pathlib.Path(value).resolve()), sys.argv[1:4])
    root = (certs.OUT / "profile-store").resolve()
    root.mkdir()
    env = dict(os.environ, ENVBOX_CONFIG_ROOT=str(root), ENVBOX_RUNTIME_DLL=dll)

    def command(*args):
        result = subprocess.run([cli, *args], env=env, text=True, capture_output=True, timeout=15)
        if result.returncode:
            raise AssertionError((args, result.stdout, result.stderr))
        return result.stdout.strip()

    tls_listener = socket.socket()
    tls_listener.bind(("127.0.0.1", 0))
    tls_listener.listen(16)
    tls_listener.settimeout(.1)
    tls_port = tls_listener.getsockname()[1]
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.bind(("127.0.0.1", 0))
    udp.settimeout(.1)
    udp_port = udp.getsockname()[1]
    stop = threading.Event()
    counts = {"tls_connections": 0, "tls_dns": 0, "udp_dns": 0}

    def tls_server():
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_2
        context.load_cert_chain(str(certs.OUT / "valid.pem"), str(certs.OUT / "valid.key"))
        while not stop.is_set():
            try:
                client, _ = tls_listener.accept()
            except socket.timeout:
                continue
            counts["tls_connections"] += 1
            try:
                with client:
                    client.settimeout(2)
                    with context.wrap_socket(client, server_side=True) as secure:
                        certs.exact(secure, struct.unpack("!H", certs.exact(secure, 2))[0])
                        counts["tls_dns"] += 1
            except (OSError, EOFError, ssl.SSLError):
                pass

    def udp_server():
        while not stop.is_set():
            try:
                query, peer = udp.recvfrom(65535)
            except socket.timeout:
                continue
            counts["udp_dns"] += 1
            # A deterministic TXT answer proves the explicitly configured
            # upstream supplied the result rather than a generic host failure.
            answer = query[:2] + b"\x81\x80\x00\x01\x00\x01\x00\x00\x00\x00" + query[12:]
            answer += b"\xc0\x0c\x00\x10\x00\x01\x00\x00\x00\x01\x00\x08\x07profile"
            udp.sendto(answer, peer)

    workers = [threading.Thread(target=tls_server), threading.Thread(target=udp_server)]
    for worker in workers:
        worker.start()
    try:
        profile = command("profile", "add", "--name", "DoT fixture", "--locale", "en-US",
            "--ui-language", "en-US", "--region", "US", "--tz-windows", "Pacific Standard Time",
            "--tz-iana", "America/Los_Angeles", "--dns-mode", "host")
        command("profile", "dns", "add", profile, "--type", "dot", "--address", "127.0.0.1",
                "--port", str(tls_port), "--server-name", "fixture.test")
        command("profile", "dns", "set", profile, "--mode", "virtual_view", "--strict", "true")
        for fallback in (False, True):
            if fallback:
                command("profile", "dns", "add", profile, "--type", "udp", "--address", "127.0.0.1",
                        "--port", str(udp_port))
            for api in ("a", "w", "utf8", "ex", "async"):
                before = dict(counts)
                text = command("run", "--profile", profile, "--audit", probe,
                               "--dns-rr", "dot.fixture.test", "16", api)
                status = field(text, "DnsRR_Status")
                records = field(text, "DnsRR_Records")
                delta = {key: counts[key] - before[key] for key in counts}
                print(json.dumps({"api": api, "fallback": fallback, "status": status,
                                  "records": records, "counts": delta}), flush=True)
                assert delta == {"tls_connections": 1, "tls_dns": 0, "udp_dns": int(fallback)}
                assert status == "0" if fallback else status != "0"
                assert records == ("1" if fallback else "0")
        audit = "\n".join(path.read_text(encoding="utf-8") for path in root.glob("audit/*.jsonl"))
        assert "dot-certificate-invalid" in audit
        assert '"dns-host"' not in audit
        try:
            control = subprocess.run([probe, "--dns-rr", "dot.fixture.test", "16", "w"],
                                     capture_output=True, text=True, timeout=10)
            print("host-control", field(control.stdout, "DnsRR_Status"), "config-root", root, flush=True)
        except subprocess.TimeoutExpired:
            # Windows resolver retry timing can exceed our bounded fixture run.
            # subprocess.run kills and waits for this owned control child. This
            # is recorded as an uncompleted Host API comparison, not a pass.
            print("host-control uncompleted: native query exceeded 10s; owned child cleaned", flush=True)
        print("PASS actual injected failure/fallback; auxiliary traffic capture not performed", flush=True)
    finally:
        stop.set()
        for worker in workers:
            worker.join(3)
        tls_listener.close()
        udp.close()
        assert not any(worker.is_alive() for worker in workers)


if __name__ == "__main__":
    main()
