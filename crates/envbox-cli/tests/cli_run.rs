//! CLI run seam: launch a process under a Profile without API hooks.

use std::process::Command;
use uuid::Uuid;

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root);
    cmd
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
