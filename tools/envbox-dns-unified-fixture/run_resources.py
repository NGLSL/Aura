"""Native same-process DNS repeat/concurrency/cancel and child fixture."""
import argparse
import ctypes
import json
import os
import pathlib
import subprocess
import sys
import uuid

sys.dont_write_bytecode = True
from run import Server, actor


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--harness64", required=True)
    parser.add_argument("--harness32", required=True)
    parser.add_argument("--runtime-dir", required=True)
    parser.add_argument("--local-only", action="store_true")
    args = parser.parse_args()
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
    kernel.GetModuleHandleW.restype = ctypes.c_void_p
    def clean():
        if any(kernel.GetModuleHandleW(name) for name in ("envbox-runtime64.dll", "envbox-runtime32.dll")):
            raise AssertionError("injected resources controller")
    clean()
    root = pathlib.Path(__file__).resolve().parents[2] / "target" / ("dns-unified-resources-" + uuid.uuid4().hex)
    root.mkdir()
    actors = {"cli": actor(args.cli), "harness64": actor(args.harness64), "harness32": actor(args.harness32)}
    actors.update({"runtime" + arch: actor(pathlib.Path(args.runtime_dir) / ("envbox-runtime" + arch + ".dll")) for arch in ("64", "32")})
    evidence = {"controller_pid": os.getpid(), "controller_runtime_modules": 0, "actors": actors, "rows": [], "pass": False,
                "boundaries": ["Short repeat batches are not production soak", "No GUI was opened or closed", "Audit is not independent global packet capture"]}
    servers = [Server("udp"), Server("tcp"), Server("tcp", silent=True)]
    def command(env, *values):
        result = subprocess.run([actors["cli"]["path"], *values], env=env, text=True, capture_output=True, timeout=30)
        if result.returncode:
            raise AssertionError((values, result.returncode, result.stdout, result.stderr))
        return result.stdout.strip()
    try:
        for arch in ("64", "32"):
            env = {key: value for key, value in os.environ.items() if not key.startswith("ENVBOX_")}
            env.update(ENVBOX_CONFIG_ROOT=str(root / ("config" + arch)), ENVBOX_RUNTIME_DLL=actors["runtime" + arch]["path"])
            upstreams = [("udp", ["--type", "udp", "--address", "127.0.0.1", "--port", str(servers[0].port)], "rr.fixture.test", "fixture"),
                         ("tcp", ["--type", "tcp", "--address", "127.0.0.1", "--port", str(servers[1].port)], "rr.fixture.test", "fixture"),
                         ("cancel", ["--type", "tcp", "--address", "127.0.0.1", "--port", str(servers[2].port)], "rr.fixture.test", "fixture")]
            if not args.local_only:
                upstreams += [("dot", ["--type", "dot", "--address", "1.1.1.1", "--port", "853", "--server-name", "cloudflare-dns.com"], "cloudflare-dns.com", "public"),
                              ("doh", ["--type", "doh", "--url", "https://cloudflare-dns.com/dns-query", "--bootstrap", "1.1.1.1", "--tls-revocation", "standard"], "cloudflare-dns.com", "public")]
            for transport, upstream, name, mode in upstreams:
                identifier = command(env, "profile", "add", "--name", transport + arch, "--locale", "en-US", "--ui-language", "en-US",
                                     "--region", "US", "--tz-windows", "Pacific Standard Time", "--tz-iana", "America/Los_Angeles", "--dns-mode", "host")
                command(env, "profile", "dns", "add", identifier, *upstream)
                command(env, "profile", "dns", "set", identifier, "--mode", "virtual_view", "--strict", "true")
                clean()
                server = {"udp": servers[0], "tcp": servers[1], "cancel": servers[2]}.get(transport)
                wire_before = len(server.wire) if server else 0
                result = subprocess.run([actors["cli"]["path"], "run", "--profile", identifier, "--audit", actors["harness" + arch]["path"],
                                         "--cancel" if transport == "cancel" else "--resources", name, mode], env=env, text=True, capture_output=True, timeout=180)
                rows = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
                runtime_facts = [actor(sample["runtime_path"]) for sample in rows if sample.get("runtime_loaded")]
                row = {"architecture": arch, "transport": transport, "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr,
                       "samples": rows, "runtime_facts": runtime_facts, "request_wire": server.wire[wire_before:] if server else None,
                       "pass": result.returncode == 0 and any(sample.get("stable") for sample in rows)
                       and bool(runtime_facts) and all(fact["sha256"] == actors["runtime" + arch]["sha256"] for fact in runtime_facts)}
                if transport != "cancel":
                    identities = [(sample["profile_id"], sample["instance_id"]) for sample in rows if sample.get("runtime_loaded")]
                    row["pass"] = row["pass"] and any(sample.get("child_pass") for sample in rows) and len(identities) == 2 and identities[0] == identities[1]
                evidence["rows"].append(row)
                print(json.dumps({key: row[key] for key in ("architecture", "transport", "exit", "pass")}), flush=True)
                (root / "result.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
        audit = "\n".join(path.read_text(encoding="utf-8") for path in root.glob("config*/audit/*.jsonl"))
        evidence["audit_host_fallback_entries"] = audit.count('"dns-host"')
        evidence["pass"] = all(row["pass"] for row in evidence["rows"]) and evidence["audit_host_fallback_entries"] == 0
    finally:
        for server in servers:
            server.close()
        clean()
        (root / "result.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
        print(json.dumps({"evidence": str(root / "result.json"), "pass": evidence["pass"]}), flush=True)
    if not evidence["pass"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
