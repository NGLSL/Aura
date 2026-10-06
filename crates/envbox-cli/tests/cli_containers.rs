use std::process::{Command, Output};

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_envbox"));
    command.env("ENVBOX_CONFIG_ROOT", root).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().unwrap()
}

#[test]
fn cli_create_list_edit_restart_and_failures_use_same_store() {
    let root = std::env::temp_dir().join(format!("aura-cli-workspace-{}", uuid::Uuid::new_v4()));
    let profile = run(
        &root,
        &[
            "profile",
            "add",
            "--name",
            "US",
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
        ],
    );
    assert!(
        profile.status.success(),
        "{}",
        String::from_utf8_lossy(&profile.stderr)
    );
    let store = envbox_storage::ConfigStore::new(&root);
    let profile_id = store.load_profiles().unwrap().profiles[0].id.to_string();
    let old_profiles = std::fs::read(store.profiles_path()).unwrap();
    let a = run(
        &root,
        &[
            "container",
            "create",
            "--name",
            "same",
            "--profile",
            &profile_id,
        ],
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let id = String::from_utf8(a.stdout).unwrap().trim().to_owned();
    let b = run(
        &root,
        &[
            "container",
            "create",
            "--name",
            "same",
            "--profile",
            &profile_id,
        ],
    );
    assert!(b.status.success());
    assert_ne!(id, String::from_utf8(b.stdout).unwrap().trim());
    let before = store.load_containers().unwrap().containers[0].clone();
    assert!(run(&root, &["container", "edit", &id, "--name", "renamed"])
        .status
        .success());
    let after = store.load_containers().unwrap().containers[0].clone();
    assert_eq!(before.id, after.id);
    assert_eq!(before.created_at_unix_ms, after.created_at_unix_ms);
    assert_eq!(after.name, "renamed");
    let list = run(&root, &["container", "list"]);
    assert!(list.status.success());
    let text = String::from_utf8(list.stdout).unwrap();
    assert!(text.contains(&id) && text.contains("renamed") && text.contains("compatibility"));
    let bytes = std::fs::read(store.containers_path()).unwrap();
    for args in [
        vec!["container", "edit", &id, "--mode", "container"],
        vec!["container", "edit", &id, "--mode", "strong"],
        vec![
            "container",
            "edit",
            &id,
            "--profile",
            "00000000-0000-0000-0000-000000000099",
        ],
        vec!["container", "edit", &id, "--unexpected", "anything"],
    ] {
        assert!(!run(&root, &args).status.success());
        assert_eq!(std::fs::read(store.containers_path()).unwrap(), bytes);
    }
    assert_eq!(std::fs::read(store.profiles_path()).unwrap(), old_profiles);
    std::fs::write(
        store.containers_path(),
        "schema_version = 99\ncontainers = []\n",
    )
    .unwrap();
    let failure = run(&root, &["container", "list"]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("schema"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_prepare_and_inspect_preserve_snapshot_after_profile_change_and_delete() {
    let root = std::env::temp_dir().join(format!("aura-cli-snapshot-{}", uuid::Uuid::new_v4()));
    assert!(run(
        &root,
        &[
            "profile",
            "add",
            "--name",
            "US",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles"
        ]
    )
    .status
    .success());
    let store = envbox_storage::ConfigStore::new(&root);
    let profile_id = store.load_profiles().unwrap().profiles[0].id.to_string();
    let created = run(
        &root,
        &[
            "container",
            "create",
            "--name",
            "A",
            "--profile",
            &profile_id,
        ],
    );
    assert!(created.status.success());
    let id = String::from_utf8(created.stdout).unwrap().trim().to_owned();
    let instance = uuid::Uuid::new_v4().to_string();
    let prepared = run(
        &root,
        &["container", "prepare", &id, "--instance", &instance],
    );
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let output = String::from_utf8(prepared.stdout).unwrap();
    assert!(output.contains("prepared_only\ttrue") && output.contains("process_started\tfalse"));
    let inspected = run(&root, &["container", "show-snapshot", &id, &instance]);
    assert!(inspected.status.success());
    let old: envbox_core::RunSnapshot =
        toml::from_str(&String::from_utf8(inspected.stdout.clone()).unwrap()).unwrap();
    old.validate().unwrap();
    let mut profiles = store.load_profiles().unwrap();
    profiles.profiles[0].name = "changed after prepare".into();
    store.save_profiles(&profiles).unwrap();
    assert!(!run(
        &root,
        &["container", "prepare", &id, "--instance", &instance]
    )
    .status
    .success());
    assert!(run(&root, &["container", "prepare", &id]).status.success());
    store
        .save_profiles(&envbox_storage::ProfileDocument { profiles: vec![] })
        .unwrap();
    assert!(run(&root, &["container", "list"]).status.success());
    assert!(!run(&root, &["container", "prepare", &id]).status.success());
    let after = run(&root, &["container", "show-snapshot", &id, &instance]);
    assert!(after.status.success());
    assert_eq!(after.stdout, inspected.stdout);
    assert!(!run(
        &root,
        &[
            "container",
            "show-snapshot",
            &uuid::Uuid::new_v4().to_string(),
            &instance
        ]
    )
    .status
    .success());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retired_policy_commands_fail_without_creating_or_mutating_configuration() {
    let root =
        std::env::temp_dir().join(format!("aura-cli-retired-policy-{}", uuid::Uuid::new_v4()));
    let id = uuid::Uuid::new_v4().to_string();
    for action in ["show", "add", "remove", "preview"] {
        let output = run(&root, &["container", "policy", action, &id]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            !root.exists(),
            "retired command must not initialize a store"
        );
    }
    std::fs::create_dir_all(&root).unwrap();
    // An unreadable document establishes that rejection precedes configuration parsing.
    let path = root.join("containers.toml");
    let historical = b"schema_version = 99\n# historical storage policy retained\n";
    std::fs::write(&path, historical).unwrap();
    for action in ["show", "add", "remove", "preview"] {
        let output = run(&root, &["container", "policy", action, &id]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), historical);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
    std::fs::remove_dir_all(root).unwrap();
}
