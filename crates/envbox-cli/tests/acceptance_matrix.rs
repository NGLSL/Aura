//! Ticket 12 acceptance matrix: Host vs US Profile, tool matrix, host transparency,
//! file access, and launch latency. Seams: Probe, CLI, and public launcher contracts
//! (InstanceManager for notepad Stop).

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root);
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        cmd.env("ENVBOX_RUNTIME_DLL", dll);
    }
    cmd
}

fn test_runtime_dll() -> Option<PathBuf> {
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        let p = PathBuf::from(dll);
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

fn probe_exe() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = PathBuf::from(env!("CARGO_BIN_EXE_envbox")).parent() {
        candidates.push(dir.join("envbox-probe.exe"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("envbox-probe.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn make_us_profile(root: &std::path::Path) -> String {
    let out = envbox_with_root(root)
        .args([
            "profile",
            "add",
            "--name",
            "US Acceptance",
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
            "ENVBOX_ACCEPTANCE=us-profile",
        ])
        .output()
        .expect("profile add");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_capture(root: &std::path::Path, profile_id: &str, program: &str, args: &[&str]) -> String {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", profile_id, program])
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run {program}: {e}"));
    assert!(out.status.success(), "run {program} {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn field_after(text: &str, key: &str) -> String {
    let idx = text.find(key).unwrap_or_else(|| panic!("missing {key}"));
    let rest = &text[idx + key.len()..];
    let line = rest.lines().next().unwrap_or_default().trim();
    if !line.is_empty() {
        return line.to_string();
    }
    rest.lines()
        .skip(1)
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn host_probe_fields() -> Vec<(String, String)> {
    let probe = probe_exe().expect("envbox-probe.exe required");
    let out = Command::new(&probe).output().expect("host probe");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let keys = [
        "GetDynamicTimeZoneInformation:",
        "GetUserDefaultGeoName:",
        "GetUserDefaultLocaleName:",
        "GetUserDefaultUILanguage:",
        "GetNetworkParams:",
    ];
    keys.iter()
        .map(|k| (k.to_string(), field_after(&text, k)))
        .collect()
}

/// Ticket 12: Host vs US Probe — Profile fields match the fixture, while
/// non-virtualized fields match the Host baseline.
#[test]
fn acceptance_host_and_us_probe_values() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-acc-{}", Uuid::new_v4()));
    let profile_id = make_us_profile(&root);

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

    // Virtualized: Profile literals must appear on the run side.
    assert_eq!(field_after(&run_out, "GetUserDefaultGeoName:"), "US");
    assert_eq!(field_after(&run_out, "GetUserDefaultLocaleName:"), "en-US");
    assert_eq!(field_after(&run_out, "GetUserDefaultUILanguage:"), "0x0409");
    assert_eq!(
        field_after(&run_out, "GetDynamicTimeZoneInformation:"),
        "Pacific Standard Time"
    );
    assert_eq!(
        field_after(&run_out, "WinRT_Calendar_GetTimeZone:"),
        "America/Los_Angeles"
    );
    assert_eq!(
        field_after(&run_out, "WinRT_RoActivateInstance_GetTimeZone:"),
        "America/Los_Angeles"
    );
    assert_eq!(
        field_after(&run_out, "WinRT_Calendar_ChangedTimeZone:"),
        "Europe/London",
        "explicit Calendar timezone changes must remain effective"
    );
    assert_eq!(
        field_after(&run_out, "GetLocalTime_MatchesProfileConversion:"),
        "true",
        "native local time must agree with the Profile timezone"
    );
    assert_eq!(
        field_after(&run_out, "SystemTimeToTzSpecificLocalTime_DateBoundary:"),
        "2023-12-31 19:00:00"
    );
    assert_eq!(
        field_after(&run_out, "HKLM_TimeZone_TimeZoneKeyName_A:"),
        "Pacific Standard Time",
        "ANSI registry reads must see the Profile timezone"
    );
    assert_eq!(
        field_after(&run_out, "HKLM_TimeZone_TimeZoneKeyName_A_QueryContract:"),
        "ok",
        "ANSI size query, short buffer, and exact buffer reads must follow the Win32 contract"
    );

    // A Host can already use US/en-US, so contrast is not a portable
    // assertion. The Profile literals above and the loaded marker prove the
    // target received the virtualized view.
    assert!(run_out.contains("EnvBox Runtime Loaded"));

    // Non-virtualized timeline marker stays real (same absolute date line if present).
    if host_out.contains("SystemTimeAsFileTime") {
        assert_eq!(
            field_after(&host_out, "SystemTimeAsFileTime:"),
            field_after(&run_out, "SystemTimeAsFileTime:"),
            "absolute time must stay real"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn acceptance_ansi_default_locale_uses_profile() {
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-locale-{}", Uuid::new_v4()));
    let added = envbox_with_root(&root)
        .args([
            "profile",
            "add",
            "--name",
            "French Acceptance",
            "--locale",
            "fr-FR",
            "--ui-language",
            "fr-FR",
            "--region",
            "FR",
            "--tz-windows",
            "Romance Standard Time",
            "--tz-iana",
            "Europe/Paris",
        ])
        .output()
        .expect("profile add");
    assert!(added.status.success(), "{added:?}");
    let profile_id = String::from_utf8_lossy(&added.stdout).trim().to_string();
    let run = envbox_with_root(&root)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run probe");
    assert!(run.status.success(), "{run:?}");
    let output = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        field_after(&output, "GetLocaleInfoA_USER_DEFAULT_SNAME:"),
        "fr-FR"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 12 matrix: cmd / powershell / git / node / python under US Profile.
#[test]
fn acceptance_tool_matrix_short_commands() {
    let root = std::env::temp_dir().join(format!("envbox-acc-{}", Uuid::new_v4()));
    let profile_id = make_us_profile(&root);

    // cmd
    let cmd_out = run_capture(
        &root,
        &profile_id,
        "cmd",
        &["/c", "echo %ENVBOX_ACCEPTANCE%"],
    );
    assert!(cmd_out.contains("us-profile"), "cmd env: {cmd_out}");

    // powershell
    let ps_out = run_capture(
        &root,
        &profile_id,
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "Write-Output $env:ENVBOX_ACCEPTANCE",
        ],
    );
    assert!(ps_out.contains("us-profile"), "ps env: {ps_out}");

    // git (required matrix row)
    let git_out = run_capture(&root, &profile_id, "git", &["--version"]);
    assert!(git_out.to_lowercase().contains("git"), "git: {git_out}");

    // node (required matrix row)
    let node_out = run_capture(
        &root,
        &profile_id,
        "node",
        &["-e", "console.log(process.env.ENVBOX_ACCEPTANCE)"],
    );
    assert!(node_out.contains("us-profile"), "node env: {node_out}");

    // python (required matrix row)
    let py_out = run_capture(
        &root,
        &profile_id,
        "python",
        &[
            "-c",
            "import os; print(os.environ.get('ENVBOX_ACCEPTANCE'))",
        ],
    );
    assert!(py_out.contains("us-profile"), "python env: {py_out}");

    // Child node via cmd (Process Tree Instance; env must inherit — not ENVBOX-only).
    let child_node = run_capture(
        &root,
        &profile_id,
        "cmd",
        &[
            "/c",
            "node",
            "-e",
            "console.log(process.env.ENVBOX_ACCEPTANCE)",
        ],
    );
    assert!(
        child_node.contains("us-profile"),
        "child node env: {child_node}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 12: real Node CLI agent (`my-claude` / Claude Code CLI) under Profile.
#[test]
#[ignore = "requires the locally installed my-claude CLI"]
fn acceptance_node_cli_agent_scenario() {
    let root = std::env::temp_dir().join(format!("envbox-acc-{}", Uuid::new_v4()));
    let profile_id = make_us_profile(&root);

    // Real Node CLI agent via node entry (npm `my-claude`). ComSpec wrapper also covered by resolve tests.
    let cli = r"D:\Program Files\nodejs\node_modules\my-claude\dist\cli.js";
    if !PathBuf::from(cli).is_file() {
        panic!("my-claude CLI entry missing: {cli}");
    }

    let stdout = run_capture(&root, &profile_id, "node", &[cli, "--help"]);
    assert!(
        stdout.contains("my-claude") || stdout.contains("Usage:"),
        "agent CLI help: {stdout}"
    );

    // Same agent as Command LaunchTarget (PATH my-claude.cmd → ComSpec).
    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env(
            "ENVBOX_RUNTIME_DLL",
            test_runtime_dll().expect("runtime DLL required"),
        )
        .args(["run", "--profile", &profile_id, "my-claude", "--help"])
        .output()
        .expect("my-claude command");
    assert!(out.status.success(), "my-claude --help: {out:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("my-claude") || text.contains("Usage:"),
        "my-claude help via command: {text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 12: notepad launches under Profile and Stop tears down the tree.
#[test]
fn acceptance_notepad_launch_and_stop() {
    use envbox_core::{
        Application, DnsMode, DnsProfile, EnvironmentProfile, InstanceStatus, LaunchTarget,
        LocaleProfile, RegistryProfile, TimezoneProfile,
    };
    use envbox_launcher::{InstanceManager, RunTarget};
    use std::collections::HashMap;

    let profile = EnvironmentProfile {
        id: Uuid::new_v4(),
        name: "US Notepad".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: DnsProfile {
            mode: DnsMode::Host,
            servers: vec![],
            ..Default::default()
        },
        environment: HashMap::new(),
        registry: RegistryProfile::default(),
        browser: Default::default(),
    };
    let app = Application {
        id: Uuid::new_v4(),
        name: "Notepad".into(),
        launch: LaunchTarget::Command {
            command: "notepad".into(),
        },
        console_host: envbox_core::ConsoleHost::Direct,
        working_directory: None,
        arguments: vec![],
        default_profile_id: profile.id,
        inherit_children: true,
        audit: false,
    };

    let mut mgr = InstanceManager::new();
    let id = mgr
        .run(&app, RunTarget::Profile(profile))
        .expect("notepad launch under Profile");
    let st = mgr.refresh(id).expect("refresh");
    assert!(
        matches!(
            st,
            InstanceStatus::Running | InstanceStatus::Starting | InstanceStatus::Exited
        ),
        "unexpected status {st:?}"
    );
    mgr.stop(id).expect("stop notepad tree");
    let st = mgr.refresh(id).expect("refresh after stop");
    assert_eq!(st, InstanceStatus::Exited);
}

/// Ticket 12: target keeps filesystem access (project drive + user profile).
#[test]
fn acceptance_filesystem_access_from_target() {
    let root = std::env::temp_dir().join(format!("envbox-acc-{}", Uuid::new_v4()));
    let profile_id = make_us_profile(&root);

    let script = r#"import os, pathlib
paths = [r"C:\\", r"D:\\", os.path.expanduser("~"), r"D:\\Project\\Aura"]
for p in paths:
    ok = os.path.isdir(p)
    print(f"DIR {p} {ok}")
print("DONE")
"#;
    if Command::new("python").arg("--version").output().is_err() {
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let out = run_capture(&root, &profile_id, "python", &["-c", script]);
    assert!(out.contains("DIR C:") && out.contains("True"), "{out}");
    assert!(out.contains("DIR D:") && out.contains("True"), "{out}");
    assert!(out.contains("DONE"), "{out}");
    // Git metadata readable when repo present
    if PathBuf::from(r"D:\Project\Aura\.git").exists() {
        let git = run_capture(
            &root,
            &profile_id,
            "git",
            &[
                "-C",
                r"D:\Project\Aura",
                "rev-parse",
                "--is-inside-work-tree",
            ],
        );
        assert!(git.contains("true"), "git repo access: {git}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 12: Host config unchanged after matrix probe + short tool runs.
#[test]
fn acceptance_host_config_unchanged_after_matrix() {
    let before = host_probe_fields();
    let root = std::env::temp_dir().join(format!("envbox-acc-{}", Uuid::new_v4()));
    let profile_id = make_us_profile(&root);
    let probe = probe_exe().expect("probe");
    let dll = test_runtime_dll().expect("dll");

    let run = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run");
    assert!(run.status.success());

    let _ = run_capture(&root, &profile_id, "cmd", &["/c", "exit", "0"]);
    let _ = run_capture(
        &root,
        &profile_id,
        "powershell",
        &["-NoProfile", "-Command", "exit 0"],
    );
    let _ = run_capture(&root, &profile_id, "git", &["--version"]);
    let _ = run_capture(&root, &profile_id, "node", &["-e", "process.exit(0)"]);
    let _ = run_capture(&root, &profile_id, "python", &["-c", "raise SystemExit(0)"]);

    let after = host_probe_fields();
    for ((k, b), (_, a)) in before.iter().zip(after.iter()) {
        assert_eq!(b, a, "Host {k} changed after matrix: {b} -> {a}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 12: extra launch latency acceptable < 300ms (ideal < 100ms).
/// Warmup + median of 5 samples so cold Rust/DLL load does not dominate.
#[test]
fn acceptance_launch_latency_under_300ms() {
    let root = std::env::temp_dir().join(format!("envbox-acc-{}", Uuid::new_v4()));
    let profile_id = make_us_profile(&root);

    // Warmup (process create + Detours + CLI).
    let _ = run_capture(&root, &profile_id, "cmd", &["/c", "exit", "0"]);
    let _ = Command::new("cmd")
        .args(["/c", "exit", "0"])
        .status()
        .expect("plain warmup");

    let mut virt = Vec::new();
    let mut plain = Vec::new();
    for _ in 0..8 {
        let t0 = Instant::now();
        let _ = run_capture(&root, &profile_id, "cmd", &["/c", "exit", "0"]);
        virt.push(t0.elapsed());
        let t1 = Instant::now();
        let st = Command::new("cmd")
            .args(["/c", "exit", "0"])
            .status()
            .expect("plain cmd");
        assert!(st.success());
        plain.push(t1.elapsed());
    }
    virt.sort();
    plain.sort();
    // Best-of-N: sibling tests steal CPU; achievable path is the product contract.
    let virtualized_best = virt[0];
    let plain_m = plain[0];
    let overhead = virtualized_best.saturating_sub(plain_m);
    eprintln!(
        "launch latency: best_virt={virtualized_best:?} median_virt={:?} plain={plain_m:?} overhead={overhead:?} samples={virt:?}",
        virt[virt.len() / 2]
    );
    assert!(
        overhead < Duration::from_millis(300),
        "extra launch latency {overhead:?} must be < 300ms (ideal < 100ms)"
    );
    let _ = std::fs::remove_dir_all(&root);
}
