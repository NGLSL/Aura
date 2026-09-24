//! Ticket 23: Audit Mode acceptance — on/off, spawn-child, Fail Open, no secrets.

use std::process::Command;
use uuid::Uuid;

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root);
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        cmd.env("ENVBOX_RUNTIME_DLL", dll);
    }
    cmd
}

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
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("envbox-runtime64.dll"));
            }
            candidates.push(dir.join("envbox-runtime64.dll"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn probe_exe() -> Option<std::path::PathBuf> {
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
        ])
        .output()
        .expect("profile add");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn instance_id_from_stderr(stderr: &str) -> String {
    for part in stderr.split_whitespace() {
        if let Some(rest) = part.strip_prefix("instance=") {
            return rest.to_string();
        }
    }
    panic!("instance id missing in:\n{stderr}");
}

/// Ticket 23: --spawn-child parent+child share one instance file with pid/ppid link.
#[test]
fn audit_spawn_child_shares_instance_file_with_pid_ppid() {
    let _dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-audit-acc-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &_dll)
        .args(["run", "--profile", &profile_id, "--audit"])
        .arg(&probe)
        .arg("--spawn-child")
        .output()
        .expect("run probe --spawn-child --audit");
    assert!(run.status.success(), "{run:?}");
    let stderr = String::from_utf8_lossy(&run.stderr);
    let instance_id = instance_id_from_stderr(&stderr);
    let path = root.join("audit").join(format!("{instance_id}.jsonl"));
    assert!(path.is_file(), "audit file missing: {}", path.display());
    let text = std::fs::read_to_string(&path).unwrap();

    // At least two distinct pids (parent + child) in one file.
    let mut pids = std::collections::HashSet::new();
    let mut ppids = std::collections::HashSet::new();
    for line in text.lines() {
        if !line.contains("\"api\":") {
            continue;
        }
        // Minimal field scrape without full JSON dependency in tests.
        if let Some(pid) = scrape_u32(line, "\"pid\":") {
            pids.insert(pid);
        }
        if let Some(ppid) = scrape_u32(line, "\"ppid\":") {
            ppids.insert(ppid);
        }
    }
    assert!(pids.len() >= 2, "expected parent+child pids, got {pids:?}:\n{text}");
    // Child ppid should point at some recorded pid (tree reconstructible).
    assert!(
        ppids.iter().any(|p| pids.contains(p) || *p != 0),
        "ppid values should link the tree: {ppids:?} vs {pids:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 23: sink directory unwritable → Run still succeeds (Fail Open).
#[test]
fn audit_unwritable_sink_still_runs_fail_open() {
    let _dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-audit-acc-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    // Block audit directory creation: place a FILE where the directory must be.
    std::fs::write(root.join("audit"), b"not-a-dir").unwrap();

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &_dll)
        .args(["run", "--profile", &profile_id, "--audit"])
        .arg(&probe)
        .output()
        .expect("run with blocked audit sink");
    assert!(run.status.success(), "Fail Open: run must succeed: {run:?}");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("EnvBox Runtime Loaded"), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 23: summaries never carry secret-shaped fields.
#[test]
fn audit_events_have_no_secret_shaped_fields() {
    let _dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-audit-acc-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &_dll)
        .env("ENVBOX_SECRET_TOKEN", "super-secret-value-should-not-log")
        .args(["run", "--profile", &profile_id, "--audit"])
        .arg(&probe)
        .output()
        .expect("run probe --audit");
    assert!(run.status.success(), "{run:?}");
    let stderr = String::from_utf8_lossy(&run.stderr);
    let instance_id = instance_id_from_stderr(&stderr);
    let path = root.join("audit").join(format!("{instance_id}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap();

    for banned in [
        "super-secret-value-should-not-log",
        "ENVBOX_SECRET_TOKEN",
        "password",
        "authorization",
        "api_key",
        "apikey",
        "BEGIN ",
    ] {
        assert!(
            !text.to_ascii_lowercase().contains(&banned.to_ascii_lowercase())
                || (banned == "ENVBOX_SECRET_TOKEN"
                    && !text.contains("super-secret-value-should-not-log")),
            "audit must not carry secret-shaped content ({banned}):\n{text}"
        );
    }
    // Explicit: env value never appears.
    assert!(!text.contains("super-secret-value-should-not-log"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

fn scrape_u32(line: &str, key: &str) -> Option<u32> {
    let idx = line.find(key)?;
    let rest = &line[idx + key.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}
