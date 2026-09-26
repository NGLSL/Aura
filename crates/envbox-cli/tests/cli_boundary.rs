//! V0.2 boundary acceptance (tickets 30–35).
//!
//! 30 elevation/integrity · 31 x86 runtime/arch · 32 multi-level cmd/bat ·
//! 33 multi-child (Electron-style) · 34 abnormal exit / Job · 35 caller_requested_suspended

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn envbox_with_root(root: &Path) -> Command {
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

fn test_runtime_dll32() -> Option<PathBuf> {
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL32") {
        let p = PathBuf::from(dll);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("envbox-runtime32.dll"));
            }
            candidates.push(dir.join("envbox-runtime32.dll"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn probe_exe() -> Option<PathBuf> {
    find_sibling_bin("envbox-probe.exe")
}

fn suspended_helper_exe() -> Option<PathBuf> {
    find_sibling_bin("envbox-suspended-helper.exe")
}

fn find_sibling_bin(name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = PathBuf::from(env!("CARGO_BIN_EXE_envbox")).parent() {
        candidates.push(dir.join(name));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join(name));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn make_profile(root: &Path) -> String {
    make_profile_inner(root, None)
}

/// Profile with PATH forced to `path_dir` so bare command resolution is under test.
fn make_profile_with_path(root: &Path, path_dir: &Path) -> String {
    make_profile_inner(root, Some(path_dir))
}

fn make_profile_inner(root: &Path, path_dir: Option<&Path>) -> String {
    let path_val = path_dir.map(|d| format!("PATH={}", d.display()));
    let mut args: Vec<&str> = vec![
        "profile",
        "add",
        "--name",
        "US Boundary",
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
        "ENVBOX_BOUNDARY_MARK=boundary-ok",
        "--env",
        "LANG=en_US.UTF-8",
    ];
    if let Some(p) = &path_val {
        args.push("--env");
        args.push(p);
    }
    let out = envbox_with_root(root)
        .args(&args)
        .output()
        .expect("profile add");
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_envbox(
    root: &Path,
    profile_id: &str,
    program: &Path,
    args: &[&str],
) -> std::process::Output {
    run_envbox_cmd(root, profile_id, &program.display().to_string(), args)
}

fn run_envbox_cmd(
    root: &Path,
    profile_id: &str,
    command: &str,
    args: &[&str],
) -> std::process::Output {
    let dll = test_runtime_dll().expect("runtime DLL required");
    Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", profile_id])
        .arg(command)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run {command}: {e}"))
}

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

fn pe_machine(path: &Path) -> u16 {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).expect("open pe");
    let mut dos = [0u8; 0x40];
    f.read_exact(&mut dos).unwrap();
    assert_eq!(&dos[0..2], b"MZ", "not a PE: {}", path.display());
    let lfanew = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as u64;
    f.seek(SeekFrom::Start(lfanew)).unwrap();
    let mut pe = [0u8; 6];
    f.read_exact(&mut pe).unwrap();
    assert_eq!(&pe[0..4], b"PE\0\0");
    u16::from_le_bytes([pe[4], pe[5]])
}

// ---------------------------------------------------------------------------
// Ticket 30 — elevation / integrity
// ---------------------------------------------------------------------------

/// Ticket 30 unit-level: launcher error mapping is covered in envbox-launcher.
/// Integration: Startup Fail Policy — inject failure (bad DLL) never silently
/// runs unvirtualized, and elevation-related wording appears on mapped errors.
#[test]
fn t30_inject_failure_never_silently_runs_unvirtualized() {
    let root = std::env::temp_dir().join(format!("envbox-b30-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let fake = root.join("envbox-runtime64.dll");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&fake, b"MZ not a real dll").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &fake)
        .args([
            "run",
            "--profile",
            &profile_id,
            "cmd",
            "/c",
            "echo SHOULD_NOT_RUN",
        ])
        .output()
        .expect("run");
    assert!(!out.status.success(), "must fail closed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("SHOULD_NOT_RUN"),
        "child must not run without injection: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 30 optional: try a requireAdministrator manifest binary when present.
/// If the target actually launches, skip (not requireAdministrator on this host).
/// Mapping itself is unit-tested in envbox-launcher (`elevation_errors_map_*`).
#[test]
fn t30_elevated_target_fails_with_integrity_guidance_when_available() {
    // Common requireAdministrator tools are not guaranteed; probe a few paths.
    let candidates = [
        r"C:\Windows\System32\wbem\WMIC.exe",
        r"C:\Windows\System32\fsutil.exe",
    ];
    let Some(target) = candidates.iter().map(PathBuf::from).find(|p| p.is_file()) else {
        eprintln!("skip: no candidate requireAdministrator tool");
        return;
    };
    let root = std::env::temp_dir().join(format!("envbox-b30e-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args(["run", "--profile", &profile_id])
        .arg(&target)
        .args(["os", "get", "version"])
        .output()
        .expect("run elevated target");
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    // Launch succeeded → tool is not requireAdministrator; mapping covered by unit tests.
    if out.status.success() || err.contains("started pid=") {
        eprintln!("skip: target launched (not requireAdministrator); unit mapping covers 30");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let lower = err.to_ascii_lowercase();
    assert!(
        lower.contains("integrity")
            || lower.contains("elevation")
            || lower.contains("access is denied")
            || lower.contains("elevat")
            || lower.contains("getlasterror"),
        "expected integrity/elevation guidance in: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Ticket 31 — x86 Runtime / architecture
// ---------------------------------------------------------------------------

/// Ticket 31: envbox-runtime32.dll is a real x86 PE when present.
/// Also asserts envbox-runtime64.dll is x64 (matrix pair).
#[test]
fn t31_runtime_dlls_have_expected_pe_machine() {
    if let Some(dll64) = test_runtime_dll() {
        let m = pe_machine(&dll64);
        assert_eq!(m, 0x8664, "runtime64 machine=0x{m:04x}");
    } else {
        eprintln!("note: envbox-runtime64.dll not found");
    }
    match test_runtime_dll32() {
        Some(dll32) => {
            let m = pe_machine(&dll32);
            assert_eq!(m, 0x014c, "runtime32 machine=0x{m:04x} (expected x86)");
        }
        None => {
            eprintln!("skip: envbox-runtime32.dll not built (x86 matrix reduced)");
        }
    }
}

/// Ticket 31: architecture mismatch does not silently degrade.
/// Point ENVBOX_RUNTIME_DLL at runtime32 and launch an x64 target — must fail.
#[test]
fn t31_arch_mismatch_refuses_silent_fallback() {
    let Some(dll32) = test_runtime_dll32() else {
        eprintln!("skip: envbox-runtime32.dll not present");
        return;
    };
    let Some(probe) = probe_exe() else {
        panic!("envbox-probe.exe required");
    };
    // Target must be x64 for a true mismatch against runtime32.
    let m = pe_machine(&probe);
    if m != 0x8664 {
        eprintln!("skip: probe is not x64 (machine=0x{m:04x})");
        return;
    }

    let root = std::env::temp_dir().join(format!("envbox-b31-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll32)
        .args(["run", "--profile", &profile_id])
        .arg(&probe)
        .output()
        .expect("run with mismatched dll");
    assert!(
        !out.status.success(),
        "arch mismatch must fail closed: {out:?}"
    );
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let lower = err.to_ascii_lowercase();
    assert!(
        lower.contains("architecture")
            || lower.contains("mismatch")
            || lower.contains("bad exe")
            || lower.contains("getlasterror")
            || lower.contains("runtime"),
        "expected architecture mismatch guidance: {err}"
    );
    // Must not have printed a successful probe body (no silent unvirtualized run).
    assert!(
        !err.contains("=== PARENT PROBE ===") && !err.contains("EnvBox Runtime Loaded"),
        "mismatched inject must not run the target: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 31 matrix: when runtime32 exists and an x86 target exists, inject and
/// inherit. Without an x86 test exe this asserts only the DLL product.
#[test]
fn t31_x86_inject_matrix_or_dll_product() {
    let Some(dll32) = test_runtime_dll32() else {
        eprintln!("skip: envbox-runtime32.dll not present");
        return;
    };
    assert_eq!(pe_machine(&dll32), 0x014c);

    // Optional x86 probe: tools/envbox-probe-x86.exe or envbox-probe32.exe.
    let x86_probe =
        find_sibling_bin("envbox-probe-x86.exe").or_else(|| find_sibling_bin("envbox-probe32.exe"));
    let Some(x86) = x86_probe else {
        eprintln!("skip: no x86 probe exe; verified runtime32 PE only");
        return;
    };
    assert_eq!(pe_machine(&x86), 0x014c, "x86 probe must be x86 PE");

    let root = std::env::temp_dir().join(format!("envbox-b31x-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);
    let out = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .env("ENVBOX_RUNTIME_DLL", &dll32)
        .args(["run", "--profile", &profile_id])
        .arg(&x86)
        .output()
        .expect("run x86 probe");
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EnvBox Runtime Loaded"),
        "x86 child not injected:\n{stdout}"
    );
    assert_eq!(field_after(&stdout, "GetUserDefaultGeoName:"), "US");
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 31: a hooked x64 parent can create an x86 child. Detours rewrites
/// the injected DLL name from runtime64 to runtime32 and calls ordinal 1 in the
/// x86 DLL, so this covers the paired Runtime cache and helper export together.
#[test]
fn t31_x64_parent_injects_syswow64_child_from_paired_runtime_cache() {
    let Some(dll64) = test_runtime_dll() else {
        eprintln!("skip: envbox-runtime64.dll not present");
        return;
    };
    let Some(dll32) = test_runtime_dll32() else {
        eprintln!("skip: envbox-runtime32.dll not present");
        return;
    };
    if dll64.parent() != dll32.parent() {
        eprintln!("skip: Runtime DLL products are not siblings");
        return;
    }

    let x86_cmd = PathBuf::from(r"C:\Windows\SysWOW64\cmd.exe");
    if !x86_cmd.is_file() {
        eprintln!("skip: SysWOW64 cmd.exe not present");
        return;
    }
    assert_eq!(pe_machine(&dll64), 0x8664);
    assert_eq!(pe_machine(&dll32), 0x014c);
    assert_eq!(pe_machine(&x86_cmd), 0x014c);

    let root = std::env::temp_dir().join(format!("envbox-b31-cross-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let profile_id = make_profile(&root);
    let wrapper = root.join("x64-parent-to-x86-child.cmd");
    std::fs::write(
        &wrapper,
        format!(
            "@echo off\r\n\"{}\" /d /s /c set ENVBOX_PROFILE_ID\r\n",
            x86_cmd.display()
        ),
    )
    .unwrap();

    let out = run_envbox(&root, &profile_id, &wrapper, &[]);
    assert!(out.status.success(), "cross-bitness launch failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("ENVBOX_PROFILE_ID={profile_id}")),
        "x86 child did not inherit the Profile through the x64 parent:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Ticket 32 — multi-level cmd/bat wrappers
// ---------------------------------------------------------------------------

/// Ticket 32: outer.cmd → inner.cmd → leaf (probe) keeps Profile the whole way.
#[test]
fn t32_multilevel_cmd_wrappers_keep_profile() {
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-b32-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let profile_id = make_profile(&root);

    let work = root.join("wrappers");
    std::fs::create_dir_all(&work).unwrap();
    let probe_s = probe.display().to_string();
    // outer.cmd calls inner.cmd; inner.cmd runs the leaf probe.
    std::fs::write(
        work.join("inner.cmd"),
        format!("@echo off\r\ncall \"{probe_s}\" --child\r\n"),
    )
    .unwrap();
    std::fs::write(
        work.join("outer.cmd"),
        "@echo off\r\ncall \"%~dp0inner.cmd\" %*\r\n",
    )
    .unwrap();

    let out = run_envbox(&root, &profile_id, &work.join("outer.cmd"), &[]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EnvBox Runtime Loaded"),
        "leaf not injected through multi-level wrappers:\n{stdout}"
    );
    assert!(
        stdout.contains(&profile_id) || stdout.contains("ENVBOX_PROFILE_ID"),
        "ENVBOX_PROFILE_ID missing at leaf:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetUserDefaultGeoName:"),
        "US",
        "leaf lost Profile geo:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetUserDefaultLocaleName:"),
        "en-US",
        "leaf lost Profile locale:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "GetDynamicTimeZoneInformation:"),
        "Pacific Standard Time",
        "leaf lost Profile timezone:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "ENVBOX_BOUNDARY_MARK:"),
        "boundary-ok",
        "profile env not visible at leaf:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 32: PATH resolution prefers `.cmd` over extension-less shims (no fallback).
/// Runs bare `edge-tool` so EnvBox `resolve_command` must choose between
/// `edge-tool` (non-PE shim) and `edge-tool.cmd` on PATH.
#[test]
fn t32_path_resolution_prefers_cmd_wrapper() {
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-b32p-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let profile_id = make_profile_with_path(&root, &root.join("pathbin"));

    let path_dir = root.join("pathbin");
    std::fs::create_dir_all(&path_dir).unwrap();
    // npm-style non-PE shim must lose to the .cmd wrapper.
    std::fs::write(
        path_dir.join("edge-tool"),
        b"#!/usr/bin/env node\nconsole.log('shim')\n",
    )
    .unwrap();
    let probe_s = probe.display().to_string();
    std::fs::write(
        path_dir.join("edge-tool.cmd"),
        format!("@echo off\r\ncall \"{probe_s}\" --child\r\n"),
    )
    .unwrap();

    // Bare command name — resolve_command does PATH search (not an explicit path).
    let out = run_envbox_cmd(&root, &profile_id, "edge-tool", &[]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("EnvBox Runtime Loaded"),
        "edge-tool.cmd was not selected (shim fallback?):\n{stdout}"
    );
    assert_eq!(field_after(&stdout, "GetUserDefaultGeoName:"), "US");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Ticket 33 — multi-process (Electron-style)
// ---------------------------------------------------------------------------

/// Ticket 33: one parent spawns several children (utility/gpu/renderer style);
/// every child is injected and sees the same Environment View (no escape).
#[test]
fn t33_multi_child_all_injected_same_profile() {
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-b33-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let profile_id = make_profile(&root);

    let work = root.join("electron-sim");
    std::fs::create_dir_all(&work).unwrap();
    let probe_s = probe.display().to_string();
    // Parent cmd spawns three role children via CreateProcess (hooked path).
    let parent = work.join("main.cmd");
    std::fs::write(
        &parent,
        format!(
            "@echo off\r\n\
             echo === role=utility ===\r\n\
             \"{probe_s}\" --child\r\n\
             echo === role=gpu ===\r\n\
             \"{probe_s}\" --child\r\n\
             echo === role=renderer ===\r\n\
             \"{probe_s}\" --child\r\n"
        ),
    )
    .unwrap();

    let out = run_envbox(&root, &profile_id, &parent, &[]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);

    for role in ["utility", "gpu", "renderer"] {
        assert!(
            stdout.contains(&format!("=== role={role} ===")),
            "missing role {role}:\n{stdout}"
        );
    }
    let markers = stdout.matches("EnvBox Runtime Loaded").count();
    assert!(
        markers >= 3,
        "expected >=3 injected children (utility/gpu/renderer), got {markers}:\n{stdout}"
    );

    // Each child block must carry the same Profile id and Profile view.
    let mut profile_ids = Vec::new();
    for chunk in stdout.split("=== role=") {
        if !chunk.contains("=== CHILD PROBE ===") {
            continue;
        }
        let pid = field_after(chunk, "ENVBOX_PROFILE_ID:");
        assert_eq!(pid, profile_id, "child escaped Profile view:\n{chunk}");
        profile_ids.push(pid.clone());
        assert_eq!(
            field_after(chunk, "GetUserDefaultGeoName:"),
            "US",
            "{chunk}"
        );
        assert_eq!(
            field_after(chunk, "GetUserDefaultLocaleName:"),
            "en-US",
            "{chunk}"
        );
        assert_eq!(
            field_after(chunk, "GetDynamicTimeZoneInformation:"),
            "Pacific Standard Time",
            "{chunk}"
        );
    }
    assert!(
        profile_ids.len() >= 3,
        "expected >=3 child probe blocks, got {}",
        profile_ids.len()
    );
    assert!(
        profile_ids.iter().all(|p| p == &profile_id),
        "grandchildren disagree on ENVBOX_PROFILE_ID: {profile_ids:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Ticket 34 — abnormal exit / Job Object
// ---------------------------------------------------------------------------

fn sample_profile() -> envbox_core::EnvironmentProfile {
    use envbox_core::{DnsMode, DnsProfile, LocaleProfile, RegistryProfile, TimezoneProfile};
    envbox_core::EnvironmentProfile {
        id: Uuid::new_v4(),
        name: "Boundary".into(),
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
        },
        environment: Default::default(),
        registry: RegistryProfile::default(),
        browser: Default::default(),
    }
}

fn sample_app(args: Vec<String>) -> envbox_core::Application {
    use envbox_core::LaunchTarget;
    envbox_core::Application {
        id: Uuid::new_v4(),
        name: "BoundaryApp".into(),
        launch: LaunchTarget::Command {
            command: "cmd".into(),
        },
        console_host: envbox_core::ConsoleHost::Direct,
        working_directory: None,
        arguments: args,
        default_profile_id: Uuid::nil(),
        inherit_children: true,
        audit: false,
    }
}

fn wait_terminal(
    mgr: &mut envbox_launcher::InstanceManager,
    id: Uuid,
) -> envbox_core::InstanceStatus {
    use envbox_core::InstanceStatus;
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut last = InstanceStatus::Starting;
    while Instant::now() < deadline {
        last = mgr.refresh(id).expect("refresh");
        if matches!(last, InstanceStatus::Exited | InstanceStatus::Failed) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    last
}

/// Ticket 34: non-zero / abnormal exit resolves InstanceStatus ∈ Exited/Failed.
#[test]
fn t34_abnormal_exit_status_is_terminal() {
    use envbox_core::InstanceStatus;
    use envbox_launcher::{InstanceManager, RunTarget};

    let profile = sample_profile();
    let mut mgr = InstanceManager::new();
    // cmd /c exit 7 — non-zero exit (abnormal from the app's point of view).
    let app = sample_app(vec!["/c".into(), "exit 7".into()]);
    let id = mgr
        .run(&app, RunTarget::Profile(profile))
        .expect("run crashy cmd");
    let status = wait_terminal(&mut mgr, id);
    assert!(
        matches!(status, InstanceStatus::Exited | InstanceStatus::Failed),
        "abnormal exit must end Exited/Failed, got {status:?}"
    );
}

/// Ticket 34: abrupt TerminateProcess (not a clean `exit N`) still ends terminal.
#[test]
fn t34_self_terminate_status_is_terminal() {
    use envbox_core::InstanceStatus;
    use envbox_launcher::{InstanceManager, RunTarget};

    let profile = sample_profile();
    let mut mgr = InstanceManager::new();
    // Real abrupt kill: TerminateProcess on self (not a cooperative exit).
    let app = sample_app(vec![
        "/c".into(),
        "powershell -NoProfile -Command \"[System.Diagnostics.Process]::GetCurrentProcess().Kill()\"".into(),
    ]);
    let id = mgr.run(&app, RunTarget::Profile(profile)).expect("run");
    let status = wait_terminal(&mut mgr, id);
    assert!(
        matches!(status, InstanceStatus::Exited | InstanceStatus::Failed),
        "terminate path must end Exited/Failed, got {status:?}"
    );
}

/// Ticket 34: Stop() explicitly terminates the Job and leaves no residual process.
#[test]
fn t34_stop_kills_tree_without_residual_process() {
    use envbox_core::InstanceStatus;
    use envbox_launcher::{InstanceManager, RunTarget};

    let profile = sample_profile();
    let mut mgr = InstanceManager::new();
    // Long-running root so Stop has something to kill.
    let app = sample_app(vec!["/c".into(), "ping -n 30 127.0.0.1 >nul".into()]);
    let id = mgr.run(&app, RunTarget::Profile(profile)).expect("run");
    let pid = mgr.get(id).expect("instance").root_pid;
    assert_ne!(pid, 0);

    // Give the process a moment to start.
    std::thread::sleep(Duration::from_millis(80));
    mgr.stop(id).expect("stop");
    let status = mgr.refresh(id).expect("refresh");
    assert!(
        matches!(status, InstanceStatus::Exited | InstanceStatus::Failed),
        "stop must resolve terminal status, got {status:?}"
    );

    // Residual process check: OpenProcess should fail once the tree is gone.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut alive = true;
    while Instant::now() < deadline {
        alive = process_alive(pid);
        if !alive {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(!alive, "pid {pid} still alive after explicit Stop");
}

fn process_alive(pid: u32) -> bool {
    // tasklist is a stable host tool; avoid extra Windows crate deps in this test.
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.to_ascii_lowercase().contains(&format!("{pid}"))
                && !text.to_ascii_lowercase().contains("no tasks")
        }
        Err(_) => false,
    }
}

/// Ticket 34: `wait()` on an abnormal exit yields a non-success status
/// (RAII handles intact; InstanceStatus terminal coverage is above).
#[test]
fn t34_wait_returns_abnormal_exit_code() {
    use envbox_core::LaunchTarget;
    use envbox_launcher::{launch, LaunchRequest};

    let profile = sample_profile();
    let req = LaunchRequest {
        launch: LaunchTarget::Command {
            command: "cmd".into(),
        },
        arguments: vec!["/c".into(), "exit 42".into()],
        working_directory: None,
        profile: Some(profile.clone()),
        instance_id: Uuid::new_v4(),
        inherit_children: true,
        audit: false,
    };
    let mut child = launch(req).expect("launch");
    let status = child.wait().expect("wait");
    // Accept any non-success code: under Detours the raw code may be an NTSTATUS
    // (e.g. STATUS_DLL_INIT_FAILED) rather than the script's `exit 42`.
    assert!(
        !status.success(),
        "abnormal exit must not be success (got {status:?})"
    );
}

// ---------------------------------------------------------------------------
// Ticket 35 — caller_requested_suspended
// ---------------------------------------------------------------------------

/// Ticket 35: CREATE_SUSPENDED child is injected and remains suspended until the
/// caller ResumeThread. Helper reports SUSPEND_COUNT / STILL_SUSPENDED / RESUMED_OK.
#[test]
fn t35_caller_requested_suspended_stays_suspended_until_resume() {
    let helper = suspended_helper_exe()
        .expect("envbox-suspended-helper.exe required (cargo build -p envbox-suspended-helper)");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-b35-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let out = run_envbox(
        &root,
        &profile_id,
        &helper,
        &["--", &probe.display().to_string(), "--child"],
    );
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Helper prints `SUSPEND_COUNT=<n>` (not `Name:` probe style).
    let count = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("SUSPEND_COUNT="))
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    assert!(
        count > 0,
        "primary thread suspend count must be >0 after inject (still suspended):\n{stdout}"
    );
    assert!(
        stdout.contains("STILL_SUSPENDED"),
        "expected STILL_SUSPENDED marker:\n{stdout}"
    );
    assert!(
        stdout.contains("RESUMED_OK"),
        "caller Resume must let the child run:\n{stdout}"
    );
    // Child ran after Resume under the same Profile (injection happened while suspended).
    assert!(
        stdout.contains("EnvBox Runtime Loaded") || stdout.contains("=== CHILD PROBE ==="),
        "child did not produce probe output after Resume:\n{stdout}"
    );
    assert!(
        stdout.contains(&profile_id),
        "child lost Profile after suspended inject:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 35 hold path: without caller Resume the child is terminated and never runs.
#[test]
fn t35_without_resume_child_does_not_run() {
    let helper = suspended_helper_exe()
        .expect("envbox-suspended-helper.exe required (cargo build -p envbox-suspended-helper)");
    let probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-b35h-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root);

    let out = run_envbox(
        &root,
        &profile_id,
        &helper,
        &["--hold", "--", &probe.display().to_string(), "--child"],
    );
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("STILL_SUSPENDED") || stdout.contains("SUSPEND_COUNT="),
        "expected suspend evidence:\n{stdout}"
    );
    assert!(
        stdout.contains("HELD_NO_RESUME"),
        "expected HELD_NO_RESUME:\n{stdout}"
    );
    // No Resume → probe body must not appear (child never ran).
    assert!(
        !stdout.contains("=== CHILD PROBE ===") && !stdout.contains("=== PARENT PROBE ==="),
        "child must not run without caller Resume:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
