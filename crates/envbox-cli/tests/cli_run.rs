//! CLI run seam: launch a process under a Profile without API hooks.

use std::process::Command;
use uuid::Uuid;

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root);
    // Prefer an explicit runtime DLL when the test harness provides one.
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        cmd.env("ENVBOX_RUNTIME_DLL", dll);
    }
    cmd
}

/// Path to envbox-runtime64.dll used by injection tests (ticket 04).
fn test_runtime_dll() -> Option<std::path::PathBuf> {
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        let p = std::path::PathBuf::from(dll);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("envbox-runtime64.dll"));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("envbox-runtime64.dll"));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn probe_exe() -> Option<std::path::PathBuf> {
    // envbox-probe is a workspace bin; look beside envbox and in target/debug.
    let mut candidates = Vec::new();
    if let Some(dir) = std::path::PathBuf::from(env!("CARGO_BIN_EXE_envbox")).parent() {
        candidates.push(dir.join("envbox-probe.exe"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("envbox-probe.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn make_profile(root: &std::path::Path) -> String {
    let out = envbox_with_root(root)
        .args([
            "profile",
            "add",
            "--name",
            "US Development",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
            "--env",
            "ENVBOX_TEST_MARKER=from-profile",
            "--env",
            "LANG=en_US.UTF-8",
        ])
        .output()
        .expect("profile add");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn run_cmd_child_sees_profile_env_and_envbox_ids() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let out = envbox_with_root(&root)
        .args([
            "run",
            "--profile",
            &profile_id,
            "cmd",
            "/c",
            "echo %ENVBOX_TEST_MARKER% %LANG% %ENVBOX_PROFILE_ID%",
        ])
        .output()
        .expect("run");
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("from-profile"),
        "profile env not visible:\n{stdout}"
    );
    assert!(stdout.contains("en_US.UTF-8"), "LANG override missing:\n{stdout}");
    assert!(
        stdout.contains(&profile_id),
        "ENVBOX_PROFILE_ID missing:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_accepts_profile_name() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let _ = make_profile(&root);
    let out = envbox_with_root(&root)
        .args([
            "run",
            "--profile",
            "US Development",
            "cmd",
            "/c",
            "echo %ENVBOX_TEST_MARKER%",
        ])
        .output()
        .expect("run");
    assert!(out.status.success(), "run by name failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("from-profile"), "stdout: {stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_missing_profile_fails_closed() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let out = envbox_with_root(&root)
        .args([
            "run",
            "--profile",
            &Uuid::new_v4().to_string(),
            "cmd",
            "/c",
            "echo should-not-run",
        ])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not found"), "stderr: {err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_unknown_command_fails() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let out = envbox_with_root(&root)
        .args([
            "run",
            "--profile",
            &profile_id,
            "definitely-not-a-real-executable-xyz",
        ])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_rejects_missing_working_directory() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let out = envbox_with_root(&root)
        .args([
            "run",
            "--profile",
            &profile_id,
            "--working-directory",
            r"Z:\envbox-does-not-exist",
            "cmd",
            "/c",
            "echo x",
        ])
        .output()
        .expect("run");
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 04: injected Runtime must print a stable marker before probe output.
#[test]
fn run_probe_prints_runtime_loaded_marker() {
    let dll = test_runtime_dll().expect(
        "envbox-runtime64.dll required for ticket 04 smoke (build via scripts/build.ps1 or set ENVBOX_TEST_RUNTIME_DLL)",
    );
    let probe = probe_exe().expect("envbox-probe.exe required for ticket 04 smoke");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run probe");
    assert!(out.status.success(), "run probe failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EnvBox Runtime Loaded"),
        "runtime marker missing (injection smoke failed):\n{stdout}"
    );
    assert!(
        stdout.contains("=== PARENT PROBE ==="),
        "probe body missing:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 04 Startup Fail Policy: missing Runtime DLL must not fall back to plain launch.
#[test]
fn run_missing_runtime_dll_fails_closed() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let missing = root.join("no-such-envbox-runtime.dll");

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &missing)
        .args([
            "run",
            "--profile",
            &profile_id,
            "cmd",
            "/c",
            "echo should-not-run",
        ])
        .output()
        .expect("run");
    assert!(
        !out.status.success(),
        "must fail when runtime DLL missing: {out:?}"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.to_ascii_lowercase().contains("runtime"),
        "stderr should mention runtime: {err}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("should-not-run"),
        "child must not start without injection: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 04: corrupt / non-DLL Runtime path must fail closed (no plain launch).
#[test]
fn run_corrupt_runtime_dll_fails_closed() {
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let profile_id = make_profile(&root);
    let fake = root.join("envbox-runtime64.dll");
    std::fs::write(&fake, b"not a pe dll").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &fake)
        .args([
            "run",
            "--profile",
            &profile_id,
            "cmd",
            "/c",
            "echo should-not-run",
        ])
        .output()
        .expect("run");
    assert!(
        !out.status.success(),
        "must fail on corrupt runtime DLL: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("should-not-run"),
        "child must not start with corrupt DLL: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 04 / US 50: Host probe snapshot must be identical before and after a Run.
#[test]
fn run_leaves_host_probe_snapshot_unchanged() {
    let Some(probe) = probe_exe() else {
        panic!("envbox-probe.exe required");
    };
    let dll = test_runtime_dll().expect("runtime DLL required");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let before = Command::new(&probe).output().expect("host probe before");
    assert!(before.status.success());

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run probe");
    assert!(run.status.success(), "{run:?}");

    let after = Command::new(&probe).output().expect("host probe after");
    assert!(after.status.success());
    let b = String::from_utf8_lossy(&before.stdout);
    let a = String::from_utf8_lossy(&after.stdout);
    // Host sections must match; Runtime Loaded line is process-local and may differ.
    for section in ["=== GEO ===", "=== LOCALE ===", "=== LANGUAGE ===", "=== TIMEZONE ===", "=== DNS ==="] {
        let slice = |s: &str| {
            let start = s.find(section).unwrap_or(0);
            let rest = &s[start..];
            let end = rest[1..]
                .find("=== ")
                .map(|i| i + 1)
                .unwrap_or(rest.len());
            rest[..end].to_string()
        };
        assert_eq!(
            slice(&b),
            slice(&a),
            "Host section {section} changed across Run"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 05: four core APIs return Profile values under `envbox run`.
/// Expected literals come from the Profile fixture (independent of Host).
/// Also asserts the fields *changed* from the Host baseline (contrast).
#[test]
fn run_probe_four_core_apis_show_profile_values() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root); // US / en-US / Pacific Standard Time

    let host = Command::new(&probe).output().expect("host probe");
    assert!(host.status.success());
    let host_out = String::from_utf8_lossy(&host.stdout).into_owned();

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run probe");
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Independent Profile fixture literals (exact, not recomputed from Host).
    assert_eq!(
        field_after(&stdout, "GetUserDefaultGeoName:"),
        "US",
        "Geo not virtualized:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetUserDefaultLocaleName:"),
        "en-US",
        "Locale not virtualized:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetUserDefaultUILanguage:"),
        "0x0409",
        "UI language not virtualized:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetDynamicTimeZoneInformation:"),
        "Pacific Standard Time",
        "Timezone not virtualized:\n{stdout}"
    );

    // Host contrast: these four fields must differ from the Host baseline
    // (or at least the run side equals the fixture, which differs from CN host).
    for key in [
        "GetUserDefaultGeoName:",
        "GetUserDefaultLocaleName:",
        "GetUserDefaultUILanguage:",
        "GetDynamicTimeZoneInformation:",
    ] {
        let host_val = field_after(&host_out, key);
        let run_val = field_after(&stdout, key);
        assert_ne!(
            host_val, run_val,
            "expected Host vs Profile contrast for {key} (host={host_val})"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 05: non-virtualized fields stay on Host baseline (independent compare).
#[test]
fn run_probe_non_virtualized_fields_match_host() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let host = Command::new(&probe).output().expect("host probe");
    assert!(host.status.success());
    let host_out = String::from_utf8_lossy(&host.stdout).into_owned();

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run probe");
    assert!(run.status.success(), "{run:?}");
    let run_out = String::from_utf8_lossy(&run.stdout);

    for key in [
        "GetSystemDefaultLocaleName:",
        "GetSystemDefaultUILanguage:",
        "GetUserGeoID:",
        "GetUserDefaultLCID:",
        "GetUserPreferredUILanguages:",
        "GetNetworkParams:",
    ] {
        assert_eq!(
            field_after(&host_out, key),
            field_after(&run_out, key),
            "non-virtualized {key} changed under Profile"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Value line following `Name:` in Probe text format.
fn field_after(text: &str, key: &str) -> String {
    let idx = text
        .find(key)
        .unwrap_or_else(|| panic!("missing key {key} in:\n{text}"));
    let rest = &text[idx + key.len()..];
    rest.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Ticket 06: probe --spawn-child parent/child share Profile view and instance id.
#[test]
fn run_probe_spawn_child_inherits_profile() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .arg("--spawn-child")
        .output()
        .expect("run probe --spawn-child");
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let parent_at = stdout.find("=== PARENT PROBE ===").expect("parent block");
    let child_at = stdout.find("=== CHILD PROBE ===").expect("child block");
    assert!(parent_at < child_at);
    let parent = &stdout[parent_at..child_at];
    let child = &stdout[child_at..];

    assert!(
        stdout.matches("EnvBox Runtime Loaded").count() >= 2,
        "parent+child runtime markers expected:\n{stdout}"
    );

    let p_inst = field_after(parent, "ENVBOX_INSTANCE_ID:");
    let c_inst = field_after(child, "ENVBOX_INSTANCE_ID:");
    assert_eq!(p_inst, c_inst, "instance id must be shared across the tree");
    assert!(!p_inst.is_empty());
    let p_prof = field_after(parent, "ENVBOX_PROFILE_ID:");
    let c_prof = field_after(child, "ENVBOX_PROFILE_ID:");
    assert_eq!(p_prof, c_prof);

    for key in [
        "GetUserDefaultGeoName:",
        "GetUserDefaultLocaleName:",
        "GetUserDefaultUILanguage:",
        "GetDynamicTimeZoneInformation:",
    ] {
        assert_eq!(
            field_after(parent, key),
            field_after(child, key),
            "parent/child mismatch for {key}"
        );
    }
    assert_eq!(field_after(child, "GetUserDefaultGeoName:"), "US");
    assert_eq!(field_after(child, "GetUserDefaultLocaleName:"), "en-US");
    assert_eq!(
        field_after(child, "GetDynamicTimeZoneInformation:"),
        "Pacific Standard Time"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 06 matrix: cmd as intermediate still yields the same Profile view.
#[test]
fn run_cmd_to_probe_child_sees_profile() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id, "cmd", "/c"])
        .arg(&probe)
        .output()
        .expect("run cmd /c probe");
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EnvBox Runtime Loaded"),
        "grandchild not injected:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetUserDefaultGeoName:"),
        "US",
        "cmd child lost Profile:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetDynamicTimeZoneInformation:"),
        "Pacific Standard Time",
        "cmd child lost timezone view:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 06: inherit_children=false — child runs unvirtualized (Host values).
#[test]
fn run_no_inherit_children_leaves_child_unhooked() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args([
            "run",
            "--profile",
            &profile_id,
            "--no-inherit-children",
        ])
        .arg(&probe)
        .arg("--spawn-child")
        .output()
        .expect("run no-inherit");
    assert!(run.status.success(), "{run:?}");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let parent_at = stdout.find("=== PARENT PROBE ===").unwrap();
    let child_at = stdout.find("=== CHILD PROBE ===").unwrap();
    let parent = &stdout[parent_at..child_at];
    let child = &stdout[child_at..];
    // Parent is still virtualized (root injection).
    assert_eq!(field_after(parent, "GetUserDefaultGeoName:"), "US");
    // Child is not injected and not virtualized (Host values).
    assert!(
        !child.contains("EnvBox Runtime Loaded"),
        "child should not be injected when inherit is off:\n{child}"
    );
    assert_ne!(
        field_after(child, "GetUserDefaultGeoName:"),
        "US",
        "child should keep Host geo when inherit is off:\n{child}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 06 matrix (subset): powershell is used when present to prove
/// CreateProcess family inheritance beyond probe/cmd. node/git/python covered
/// in ticket 12 acceptance matrix.
#[test]
fn run_powershell_child_sees_profile_when_available() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let ps = ["powershell.exe", "pwsh.exe"]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
        .or_else(|| {
            std::env::var_os("SystemRoot").map(|root| {
                std::path::PathBuf::from(root)
                    .join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe")
            })
        });
    let Some(ps) = ps.filter(|p| p.is_file()) else {
        eprintln!("skip: powershell not present");
        return;
    };
    let root = std::env::temp_dir().join(format!("envbox-run-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    // powershell -NoProfile -Command <probe>
    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&ps)
        .args(["-NoProfile", "-Command"])
        .arg(&probe)
        .output()
        .expect("run powershell probe");
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EnvBox Runtime Loaded"),
        "powershell child not injected:\n{stdout}"
    );
    assert_eq!(field_after(&stdout, "GetUserDefaultGeoName:"), "US");
    let _ = std::fs::remove_dir_all(&root);
}
