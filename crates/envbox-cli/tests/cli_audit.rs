//! Ticket 22: `envbox audit show` / `envbox audit export` CLI seam.

use std::process::Command;
use uuid::Uuid;

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root);
    cmd
}

fn write_audit_fixture(root: &std::path::Path, instance_id: &Uuid, lines: &[&str]) {
    let dir = root.join("audit");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{instance_id}.jsonl"));
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
}

fn valid_line(api: &str) -> String {
    format!(
        r#"{{"v":1,"ts_utc":"2026-09-24T00:00:00.000Z","pid":1,"ppid":0,"tid":1,"api":"{api}","virtualized":true,"summary":"x"}}"#
    )
}

#[test]
fn audit_show_prints_valid_jsonl() {
    let root = std::env::temp_dir().join(format!("envbox-audit-cli-{}", Uuid::new_v4()));
    let id = Uuid::new_v4();
    write_audit_fixture(
        &root,
        &id,
        &[
            &valid_line("GetUserDefaultLocaleName"),
            &valid_line("GetNetworkParams"),
        ],
    );
    let out = envbox_with_root(&root)
        .args(["audit", "show", &id.to_string()])
        .output()
        .expect("audit show");
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"api\":\"GetUserDefaultLocaleName\""), "{text}");
    assert!(text.contains("\"api\":\"GetNetworkParams\""), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn audit_show_summary_counts_apis() {
    let root = std::env::temp_dir().join(format!("envbox-audit-cli-{}", Uuid::new_v4()));
    let id = Uuid::new_v4();
    write_audit_fixture(
        &root,
        &id,
        &[&valid_line("GetNetworkParams"), &valid_line("GetNetworkParams")],
    );
    let out = envbox_with_root(&root)
        .args(["audit", "show", &id.to_string(), "--summary"])
        .output()
        .expect("audit show --summary");
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("GetNetworkParams"), "{text}");
    assert!(text.contains("events\t2"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn audit_show_missing_file_fails_closed() {
    let root = std::env::temp_dir().join(format!("envbox-audit-cli-{}", Uuid::new_v4()));
    let id = Uuid::new_v4();
    let out = envbox_with_root(&root)
        .args(["audit", "show", &id.to_string()])
        .output()
        .expect("audit show missing");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not found"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn audit_show_rejects_invalid_line() {
    let root = std::env::temp_dir().join(format!("envbox-audit-cli-{}", Uuid::new_v4()));
    let id = Uuid::new_v4();
    write_audit_fixture(&root, &id, &["not-json"]);
    let out = envbox_with_root(&root)
        .args(["audit", "show", &id.to_string()])
        .output()
        .expect("audit show bad line");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn audit_export_merges_instances_to_out() {
    let root = std::env::temp_dir().join(format!("envbox-audit-cli-{}", Uuid::new_v4()));
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    write_audit_fixture(&root, &a, &[&valid_line("GetUserDefaultGeoName")]);
    write_audit_fixture(&root, &b, &[&valid_line("GetNetworkParams")]);
    let out_path = root.join("merged.jsonl");
    let out = envbox_with_root(&root)
        .args([
            "audit",
            "export",
            "--out",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("audit export");
    assert!(out.status.success(), "{out:?}");
    let text = std::fs::read_to_string(&out_path).unwrap();
    assert!(text.contains("GetUserDefaultGeoName"), "{text}");
    assert!(text.contains("GetNetworkParams"), "{text}");
    assert_eq!(text.lines().count(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn audit_export_empty_dir_fails_closed() {
    let root = std::env::temp_dir().join(format!("envbox-audit-cli-{}", Uuid::new_v4()));
    std::fs::create_dir_all(root.join("audit")).unwrap();
    let out = envbox_with_root(&root)
        .args(["audit", "export"])
        .output()
        .expect("audit export empty");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let _ = std::fs::remove_dir_all(&root);
}
