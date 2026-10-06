"""Independent repeats of failed public-service cases; never edits initial evidence."""
import argparse
import ctypes
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time
import tomllib
import uuid

sys.dont_write_bytecode = True
from run import actor, field


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence")
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args()
    initial = json.loads(pathlib.Path(args.evidence).read_text(encoding="utf-8"))
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetModuleHandleW.argtypes = [ctypes.c_wchar_p]
    kernel.GetModuleHandleW.restype = ctypes.c_void_p
    def clean():
        if any(kernel.GetModuleHandleW(name) for name in ("envbox-runtime64.dll", "envbox-runtime32.dll")):
            raise AssertionError("injected repeat controller")
    clean()
    origin = pathlib.Path(initial["config_root"])
    root = origin.parent / ("dns-unified-repeat-" + uuid.uuid4().hex)
    root.mkdir()
    output = {"controller_pid": os.getpid(), "controller_runtime_modules": 0,
              "initial_evidence": str(pathlib.Path(args.evidence).resolve()), "actors": initial["actors"], "rows": []}
    def verify_actors(arch):
        facts = {key: actor(initial["actors"][key]["path"]) for key in ("cli", "probe" + arch, "runtime" + arch)}
        for key, fact in facts.items():
            expected = initial["actors"][key]
            if fact["sha256"] != expected["sha256"] or os.path.normcase(fact["path"]) != os.path.normcase(expected["path"]):
                output["provenance_error"] = {"actor": key, "expected": expected, "actual": fact}
                (root / "result.json").write_text(json.dumps(output, indent=2), encoding="utf-8")
                raise ValueError(f"Provenance drift for frozen {key}: expected {expected['sha256']}, actual {fact['sha256']}")
        return facts
    failed_rows = [row for row in initial["rows"] if not row["pass"]]
    if not failed_rows:
        raise ValueError("Initial evidence has no failed case to repeat")
    if args.repetitions < 1:
        raise ValueError("Repetitions must be positive")
    for row in failed_rows:
        if row["architecture"] not in ("64", "32"):
            raise ValueError(f"Unsupported architecture {row['architecture']} in failed case")
    for arch in ("64", "32"):
        config = root / ("config" + arch)
        config.mkdir()
        shutil.copyfile(origin / ("config" + arch) / "profiles.toml", config / "profiles.toml")
        profiles = tomllib.loads((config / "profiles.toml").read_text(encoding="utf-8"))["profiles"]
        env = {key: value for key, value in os.environ.items() if not key.startswith("ENVBOX_")}
        env.update(ENVBOX_CONFIG_ROOT=str(config), ENVBOX_RUNTIME_DLL=initial["actors"]["runtime" + arch]["path"])
        for row in initial["rows"]:
            if row["pass"] or row["architecture"] != arch:
                continue
            identifier = row.get("profile_id")
            if identifier:
                if not any(profile["id"] == identifier for profile in profiles):
                    raise ValueError(f"Recorded Profile {identifier} not found for {row['transport']} architecture {arch}")
            else:
                legacy_names = {"udp": "udp", "tcp": "tcp", "dot": "dot", "doh": "doh",
                                "tcp-fail-udp-success": "ordered", "all-failed": "dead",
                                "tcp-cancel": "stalled", "tcp-deadline": "stalled"}
                if row["transport"] not in legacy_names:
                    raise ValueError(f"Legacy evidence has no Profile mapping for transport {row['transport']} architecture {arch}")
                profile_name = legacy_names[row["transport"]] + arch
                matches = [profile["id"] for profile in profiles if profile["name"] == profile_name]
                if len(matches) != 1:
                    raise ValueError(f"Legacy Profile {profile_name} expected once, found {len(matches)}")
                identifier = matches[0]
            default_status = {"all-failed": ["1460"], "tcp-deadline": ["1460"], "tcp-cancel": ["1223"]}
            expected = row.get("expected_status", default_status.get(row["transport"], ["0", "9501", "9003"]))
            for attempt in range(1, args.repetitions + 1):
                clean()
                actual_before = verify_actors(arch)
                audit_before = set(config.glob("audit/*.jsonl"))
                start = time.monotonic()
                values = [initial["actors"]["cli"]["path"], "run", "--profile", identifier,
                          "--audit", initial["actors"]["probe" + arch]["path"], "--dns-rr", row["name"], str(row["qtype"]), row["api"]]
                if row["transport"] == "tcp-cancel":
                    values += ["264", "--cancel"]
                result = subprocess.run(values, env=env, capture_output=True, text=True, timeout=30)
                clean()
                actual_after = verify_actors(arch)
                audit_paths = sorted(set(config.glob("audit/*.jsonl")) - audit_before)
                audit_entries = [json.loads(line) for path in audit_paths for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
                host_fallback = sum("dns-host" in entry.get("summary", "") for entry in audit_entries)
                repeated = {key: row[key] for key in ("architecture", "transport", "name", "qtype", "api")}
                repeated.update(attempt=attempt, exit=result.returncode, seconds=time.monotonic() - start,
                                status=field(result.stdout, "DnsRR_Status"), stdout=result.stdout, stderr=result.stderr)
                repeated["profile_id"] = identifier
                repeated["expected_status"] = expected
                repeated["actual_actors_before"] = actual_before
                repeated["actual_actors_after"] = actual_after
                repeated["runtime_loaded"] = "EnvBox Runtime Loaded" in result.stdout
                repeated["audit_paths"] = [str(path) for path in audit_paths]
                repeated["audit_entries"] = len(audit_entries)
                repeated["audit_host_fallback_entries"] = host_fallback
                repeated["pass"] = (result.returncode == 0 and repeated["status"] in expected and field(result.stdout, "DnsRR_Freed") == "true"
                                    and repeated["runtime_loaded"] and bool(audit_entries) and host_fallback == 0)
                output["rows"].append(repeated)
                print(json.dumps({key: repeated[key] for key in ("architecture", "qtype", "api", "attempt", "status", "pass")}), flush=True)
                (root / "result.json").write_text(json.dumps(output, indent=2), encoding="utf-8")
    output["pass"] = all(row["pass"] for row in output["rows"])
    (root / "result.json").write_text(json.dumps(output, indent=2), encoding="utf-8")
    print(json.dumps({"evidence": str(root / "result.json"), "pass": output["pass"], "count": len(output["rows"])}), flush=True)
    if not output["pass"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
