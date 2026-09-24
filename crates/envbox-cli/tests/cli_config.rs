//! CLI process I/O seam: define Profile/Application and list them back.

use std::process::Command;
use uuid::Uuid;

fn envbox() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    // Isolate config from the developer machine and from sibling tests.
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    cmd.env("ENVBOX_CONFIG_ROOT", &root);
    cmd
}

fn cleanup_root(root: &str) {
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn profile_add_then_list_shows_stable_id() {
    let mut cmd = envbox();
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    cmd.env("ENVBOX_CONFIG_ROOT", &root);

    let add = cmd
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
            "--dns-mode",
            "virtual_view",
            "--dns",
            "1.1.1.1",
            "--env",
            "LANG=en_US.UTF-8",
        ])
        .output()
        .expect("run profile add");
    assert!(add.status.success(), "add failed: {add:?}");
    let id = String::from_utf8_lossy(&add.stdout).trim().to_string();
    assert!(!id.is_empty(), "expected printed profile id");

    let list = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args(["profile", "list"])
        .output()
        .expect("list");
    assert!(list.status.success());
    let out = String::from_utf8_lossy(&list.stdout);
    assert!(out.contains(&id), "list missing id {id}:\n{out}");
    assert!(out.contains("US Development"));
    assert!(out.contains("en-US"));
    assert!(out.contains("Pacific Standard Time"));
    assert!(out.contains("virtual_view"));
    cleanup_root(&root.to_string_lossy());
}

#[test]
fn profile_add_rejects_bad_region() {
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    let add = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args([
            "profile",
            "add",
            "--name",
            "Bad",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "USA",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
        ])
        .output()
        .expect("run");
    assert!(!add.status.success());
    let err = String::from_utf8_lossy(&add.stderr);
    assert!(err.contains("region"), "stderr: {err}");
    cleanup_root(&root.to_string_lossy());
}

#[test]
fn profile_add_rejects_unknown_timezone() {
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    let add = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args([
            "profile",
            "add",
            "--name",
            "BadTz",
            "--locale",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Not A Real Zone",
            "--tz-iana",
            "America/Los_Angeles",
        ])
        .output()
        .expect("run");
    assert!(!add.status.success());
    cleanup_root(&root.to_string_lossy());
}

#[test]
fn app_add_requires_existing_profile_and_lists() {
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    let add_p = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
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
        ])
        .output()
        .expect("profile add");
    assert!(add_p.status.success(), "{add_p:?}");
    let profile_id = String::from_utf8_lossy(&add_p.stdout).trim().to_string();

    let add_a = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args([
            "app",
            "add",
            "--name",
            "Claude Code",
            "--command",
            "claude",
            "--profile",
            &profile_id,
            "--working-directory",
            "D:\\Projects",
        ])
        .output()
        .expect("app add");
    assert!(add_a.status.success(), "app add failed: {add_a:?}");
    let app_id = String::from_utf8_lossy(&add_a.stdout).trim().to_string();

    let list = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args(["app", "list"])
        .output()
        .expect("app list");
    let out = String::from_utf8_lossy(&list.stdout);
    assert!(out.contains(&app_id));
    assert!(out.contains("Claude Code"));
    assert!(out.contains("cmd=claude"));
    assert!(out.contains(&profile_id));
    cleanup_root(&root.to_string_lossy());
}

#[test]
fn app_add_rejects_unknown_profile_id() {
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    let add = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args([
            "app",
            "add",
            "--name",
            "X",
            "--command",
            "x",
            "--profile",
            &Uuid::new_v4().to_string(),
        ])
        .output()
        .expect("run");
    assert!(!add.status.success());
    cleanup_root(&root.to_string_lossy());
}

#[test]
fn profile_add_rejects_empty_locale() {
    let root = std::env::temp_dir().join(format!("envbox-cli-test-{}", Uuid::new_v4()));
    let add = Command::new(env!("CARGO_BIN_EXE_envbox"))
        .env("ENVBOX_CONFIG_ROOT", &root)
        .args([
            "profile",
            "add",
            "--name",
            "NoLocale",
            "--locale",
            "",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
        ])
        .output()
        .expect("run");
    assert!(!add.status.success());
    let err = String::from_utf8_lossy(&add.stderr);
    assert!(err.contains("locale"), "stderr: {err}");
    cleanup_root(&root.to_string_lossy());
}
