"""Run the production DoT path through the real Runtime on public endpoints.

This intentionally uses the current Windows trust/revocation policy.  It does
not import a certificate, alter the host DNS configuration, or add an
unconfigured fallback.  The fixture is a positive product-path probe only;
the process-owned endpoint is recorded by the caller when packet evidence is
needed.
"""

import ctypes
from ctypes import wintypes
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


RUNTIME_NAMES = ("envbox-runtime64.dll", "envbox-runtime32.dll")
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
TH32CS_SNAPPROCESS = 0x00000002
LIST_MODULES_ALL = 0x03


class _ProcessEntry32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD),
        ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", wintypes.DWORD),
        ("cntThreads", wintypes.DWORD),
        ("th32ParentProcessID", wintypes.DWORD),
        ("pcPriClassBase", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
        ("szExeFile", wintypes.WCHAR * 260),
    ]


def _sha256(path):
    digest = hashlib.sha256()
    with pathlib.Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest().upper()


def _actor(path):
    resolved = pathlib.Path(path).resolve(strict=True)
    return {"path": str(resolved), "sha256": _sha256(resolved),
            "size": resolved.stat().st_size}


def _module_facts(paths):
    facts = []
    for path in paths:
        resolved = pathlib.Path(path).resolve()
        facts.append({"path": str(resolved),
                      "sha256": _sha256(resolved) if resolved.is_file() else None})
    return facts


def _win32():
    if os.name != "nt":
        raise RuntimeError("the public injected fixture requires Windows")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel32.GetModuleHandleW.argtypes = [wintypes.LPCWSTR]
    kernel32.GetModuleHandleW.restype = wintypes.HMODULE
    kernel32.GetModuleFileNameW.argtypes = [wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD]
    kernel32.GetModuleFileNameW.restype = wintypes.DWORD
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry32W)]
    kernel32.Process32FirstW.restype = wintypes.BOOL
    kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry32W)]
    kernel32.Process32NextW.restype = wintypes.BOOL
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    psapi.EnumProcessModulesEx.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(wintypes.HMODULE), wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD), wintypes.DWORD,
    ]
    psapi.EnumProcessModulesEx.restype = wintypes.BOOL
    psapi.GetModuleFileNameExW.argtypes = [
        wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD,
    ]
    psapi.GetModuleFileNameExW.restype = wintypes.DWORD
    return kernel32, psapi


def _module_file_name(kernel32, module):
    buffer = ctypes.create_unicode_buffer(32768)
    length = kernel32.GetModuleFileNameW(module, buffer, len(buffer))
    return pathlib.Path(buffer.value).resolve() if length else None


def _controller_runtime_modules(kernel32):
    """Return Runtime modules loaded in this Python controller process."""
    modules = []
    for name in RUNTIME_NAMES:
        handle = kernel32.GetModuleHandleW(name)
        if handle:
            path = _module_file_name(kernel32, handle)
            modules.append(str(path) if path else name)
    return modules


def _assert_controller_clean(kernel32, stage):
    modules = _controller_runtime_modules(kernel32)
    if modules:
        raise RuntimeError(f"{stage}: controller already has Runtime loaded: {modules}")
    return modules


def _processes(kernel32):
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    invalid = ctypes.c_void_p(-1).value
    if snapshot in (None, 0, invalid):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        entry = _ProcessEntry32W()
        entry.dwSize = ctypes.sizeof(entry)
        if not kernel32.Process32FirstW(snapshot, ctypes.byref(entry)):
            return
        while True:
            yield {"pid": int(entry.th32ProcessID),
                   "parent_pid": int(entry.th32ParentProcessID),
                   "exe": entry.szExeFile}
            if not kernel32.Process32NextW(snapshot, ctypes.byref(entry)):
                break
    finally:
        kernel32.CloseHandle(snapshot)


def _process_modules(kernel32, psapi, pid):
    access = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ
    handle = kernel32.OpenProcess(access, False, pid)
    if not handle:
        return []
    try:
        capacity = 256
        count = 0
        while capacity <= 4096:
            values = (wintypes.HMODULE * capacity)()
            needed = wintypes.DWORD()
            if not psapi.EnumProcessModulesEx(handle, values,
                                              ctypes.sizeof(values), ctypes.byref(needed),
                                              LIST_MODULES_ALL):
                return []
            count = needed.value // ctypes.sizeof(wintypes.HMODULE)
            if count < capacity:
                break
            capacity *= 2
        result = []
        for module in values[:count]:
            buffer = ctypes.create_unicode_buffer(32768)
            length = psapi.GetModuleFileNameExW(handle, module, buffer, len(buffer))
            if length:
                result.append(str(pathlib.Path(buffer.value).resolve()))
        return result
    finally:
        kernel32.CloseHandle(handle)


def _probe_observation(kernel32, psapi, expected_probe):
    expected = pathlib.Path(expected_probe).resolve()
    expected_key = os.path.normcase(str(expected))
    for process in _processes(kernel32):
        if process["exe"].lower() != expected.name.lower():
            continue
        modules = _process_modules(kernel32, psapi, process["pid"])
        if not modules:
            continue
        if not any(os.path.normcase(path) == expected_key for path in modules):
            continue
        runtime = [path for path in modules
                   if pathlib.Path(path).name.lower() in RUNTIME_NAMES]
        return {"pid": process["pid"], "parent_pid": process["parent_pid"],
                "modules": modules, "runtime_modules": runtime}
    return None


def _write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n",
                    encoding="utf-8")


def _run_observed(argv, env, expected_probe, kernel32, psapi, timeout):
    """Run a process without pipe backpressure while observing its modules."""
    stdout_file = tempfile.TemporaryFile()
    stderr_file = tempfile.TemporaryFile()
    process = subprocess.Popen(argv, env=env, stdout=stdout_file, stderr=stderr_file)
    observation = None
    timed_out = False
    deadline = time.monotonic() + timeout
    try:
        while process.poll() is None:
            observation = _probe_observation(kernel32, psapi, expected_probe) or observation
            if time.monotonic() >= deadline:
                timed_out = True
                process.kill()
                break
            time.sleep(0.01)
        process.wait(timeout=10)
        observation = _probe_observation(kernel32, psapi, expected_probe) or observation
        stdout_file.seek(0)
        stderr_file.seek(0)
        stdout = stdout_file.read().decode("utf-8", "replace")
        stderr = stderr_file.read().decode("utf-8", "replace")
        return process, observation, stdout, stderr, timed_out
    finally:
        stdout_file.close()
        stderr_file.close()


def field(text, key):
    marker = key + ":\n"
    return text.split(marker, 1)[1].splitlines()[0] if marker in text else None


def main():
    if len(sys.argv) not in (4, 6):
        raise SystemExit("usage: run_public_injected.py CLI PROBE RUNTIME_DLL [IP SERVER_NAME]")
    cli, probe, dll = [str(pathlib.Path(value).resolve()) for value in sys.argv[1:4]]
    address, server_name = (sys.argv[4:6] if len(sys.argv) == 6 else
                            ("1.1.1.1", "cloudflare-dns.com"))
    kernel32, psapi = _win32()
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    target_root = (repo_root / "target").resolve()
    target_root.mkdir(parents=True, exist_ok=True)
    inherited = sorted(key for key in os.environ if key.startswith("ENVBOX_"))
    clean_environment = {key: value for key, value in os.environ.items()
                         if not key.startswith("ENVBOX_")}
    actors = {"cli": _actor(cli), "probe": _actor(probe), "runtime_dll": _actor(dll)}
    controller_before = _assert_controller_clean(kernel32, "fresh-host-before")
    root = pathlib.Path(tempfile.mkdtemp(prefix="envbox-dot-public-",
                                          dir=str(target_root))).resolve()
    if not root.is_relative_to(target_root):
        raise RuntimeError(f"temporary config escaped target: {root}")
    env = dict(clean_environment, ENVBOX_CONFIG_ROOT=str(root), ENVBOX_RUNTIME_DLL=dll)
    host_clean_env = dict(clean_environment)
    run_id = uuid.uuid4().hex
    evidence_path = target_root / f"dot-public-injected-evidence-{run_id}.json"
    evidence = {
        "run_id": run_id,
        "attempt": 1,
        "retry": False,
        "address": address,
        "server_name": server_name,
        "actors": actors,
        "environment": {
            "cleared_inherited_envbox_keys": inherited,
            "overrides": {"ENVBOX_CONFIG_ROOT": str(root), "ENVBOX_RUNTIME_DLL": dll},
        },
        "fresh_host": {
            "controller_pid": os.getpid(),
            "controller_executable": _actor(sys.executable),
            "runtime_modules_before": controller_before,
        },
        "probe_runs": [],
        "pass": False,
    }

    def command(*args):
        _assert_controller_clean(kernel32, f"before {' '.join(args)}")
        result = subprocess.run([cli, *args], env=env, text=True,
                                capture_output=True, timeout=30)
        _assert_controller_clean(kernel32, f"after {' '.join(args)}")
        if result.returncode:
            raise AssertionError((args, result.stdout, result.stderr))
        return result.stdout.strip()

    def host_probe():
        """Run an uninjected snapshot probe without DNS or Runtime env."""
        _assert_controller_clean(kernel32, "before fresh-host-probe")
        process, observation, stdout, stderr, timed_out = _run_observed(
            [probe], host_clean_env, probe, kernel32, psapi, timeout=30)
        if "EnvBox Runtime Loaded" in stdout:
            raise AssertionError("fresh host probe unexpectedly reports Runtime Loaded")
        evidence["fresh_host"].update({
            "probe_pid": process.pid,
            "probe_exit": process.returncode,
            "probe_timed_out": timed_out,
            "probe_stdout": stdout,
            "probe_stderr": stderr,
            "probe_modules": observation["modules"] if observation else [],
            "probe_runtime_modules": observation["runtime_modules"] if observation else [],
            "probe_runtime_module_facts": _module_facts(
                observation["runtime_modules"] if observation else []),
            "runtime_modules_after": _assert_controller_clean(kernel32, "after fresh-host-probe"),
        })
        if timed_out or process.returncode:
            raise AssertionError(("fresh host probe", stdout, stderr))
        if observation is None:
            raise AssertionError("fresh host probe exited before module inspection")
        if observation["runtime_modules"]:
            raise AssertionError(("fresh host probe has Runtime modules",
                                  observation["runtime_modules"]))

    def run_probe(api):
        args = ["run", "--profile", profile, "--audit", probe,
                "--dns-rr", server_name, "16", api]
        _assert_controller_clean(kernel32, f"before probe {api}")
        process, observation, stdout, stderr, timed_out = _run_observed(
            [cli, *args], env, probe, kernel32, psapi, timeout=60)
        row = {
            "api": api,
            "exit": process.returncode,
            "timed_out": timed_out,
            "status": field(stdout, "DnsRR_Status"),
            "records": field(stdout, "DnsRR_Records"),
            "runtime_marker": "EnvBox Runtime Loaded" in stdout,
            "runtime_identity_lines": [line for line in stdout.splitlines()
                                       if "runtime" in line.lower() or "module" in line.lower()],
            "probe_stdout": stdout,
            "probe_stderr": stderr,
            "probe_pid": observation["pid"] if observation else process.pid,
            "probe_parent_pid": observation["parent_pid"] if observation else None,
            "probe_modules": observation["modules"] if observation else [],
            "probe_runtime_modules": observation["runtime_modules"] if observation else [],
            "probe_runtime_module_facts": _module_facts(
                observation["runtime_modules"] if observation else []),
            "expected_runtime_sha256": actors["runtime_dll"]["sha256"],
        }
        evidence["probe_runs"].append(row)
        _assert_controller_clean(kernel32, f"after probe {api}")
        if timed_out or process.returncode or not row["runtime_marker"]:
            raise AssertionError((api, stdout, stderr, row["probe_modules"]))
        if not row["probe_runtime_modules"]:
            raise AssertionError((api, "Probe did not expose a Runtime module", row))
        if not any(fact["sha256"] == actors["runtime_dll"]["sha256"]
                   for fact in row["probe_runtime_module_facts"]):
            raise AssertionError((api, "Probe Runtime module hash differs from requested DLL", row))
        if row["status"] != "0":
            raise AssertionError((api, stdout))

    cleanup_error = None
    controller_error = None
    try:
        host_probe()
        profile = command("profile", "add", "--name", "Public DoT fixture",
            "--locale", "en-US", "--ui-language", "en-US", "--region", "US",
            "--tz-windows", "Pacific Standard Time", "--tz-iana", "America/Los_Angeles",
            "--dns-mode", "host")
        command("profile", "dns", "add", profile, "--type", "dot",
                "--address", address, "--port", "853", "--server-name", server_name)
        command("profile", "dns", "set", profile, "--mode", "virtual_view", "--strict", "true")
        for api in ("a", "w", "utf8", "ex", "async"):
            run_probe(api)
        evidence["pass"] = True
        summary = [{key: row[key] for key in (
            "api", "exit", "status", "records", "runtime_marker",
            "probe_pid", "probe_runtime_modules")}
                   for row in evidence["probe_runs"]]
        print(json.dumps({"pass": True, "address": address,
                          "server_name": server_name, "apis": summary,
                          "evidence": str(evidence_path)}), flush=True)
    finally:
        try:
            evidence["fresh_host"]["runtime_modules_final"] = _assert_controller_clean(
                kernel32, "final-controller-check")
        except Exception as error:
            controller_error = error
            evidence["fresh_host"]["runtime_modules_final_error"] = repr(error)
        evidence["cleanup"] = {"root": str(root), "removed": False, "error": None}
        _write_json(evidence_path, evidence)
        if root.exists():
            if not root.is_relative_to(target_root):
                raise RuntimeError(f"cleanup path escaped target: {root}")
            try:
                shutil.rmtree(root)
            except Exception as error:
                cleanup_error = repr(error)
                evidence["cleanup"]["error"] = cleanup_error
                _write_json(evidence_path, evidence)
                raise
            evidence["cleanup"]["removed"] = True
            _write_json(evidence_path, evidence)
        if cleanup_error:
            raise RuntimeError(cleanup_error)
        if controller_error:
            raise controller_error


if __name__ == "__main__":
    main()
