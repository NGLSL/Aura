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
    let application_path = root
        .with_extension("application")
        .to_string_lossy()
        .into_owned();
    for (suffix, action) in [
        ("", "isolated_write"),
        ("\\cache", "shared_read_only"),
        ("\\export", "shared_read_write"),
        ("\\private", "deny"),
    ] {
        let path = format!("{application_path}{suffix}");
        let result = run(
            &root,
            &[
                "container",
                "policy",
                "add",
                &id,
                "--target",
                "file",
                "--path",
                &path,
                "--action",
                action,
            ],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let show = run(&root, &["container", "policy", "show", &id]);
    let text = String::from_utf8(show.stdout).unwrap();
    assert!(text.contains("backend\tunsupported") && text.contains("host_write_exception=true"));
    let preview = run(
        &root,
        &[
            "container",
            "policy",
            "preview",
            &id,
            "--target",
            "file",
            "--path",
            &format!("{application_path}\\export\\data"),
        ],
    );
    assert!(preview.status.success());
    let text = String::from_utf8(preview.stdout).unwrap();
    assert!(text.contains("Some(SharedReadWrite)") && text.contains("can_authorize\tfalse"));
    assert!(store.load_containers().unwrap().containers[1]
        .storage_policy
        .rules
        .is_empty());
    let policy_bytes = std::fs::read(store.containers_path()).unwrap();
    let alias = application_path.replace('\\', "/").to_ascii_uppercase();
    assert!(!run(
        &root,
        &[
            "container",
            "policy",
            "add",
            &id,
            "--target",
            "file",
            "--path",
            &alias,
            "--action",
            "shared_read_write"
        ]
    )
    .status
    .success());
    assert_eq!(
        std::fs::read(store.containers_path()).unwrap(),
        policy_bytes
    );
    assert!(!run(
        &root,
        &[
            "container",
            "policy",
            "add",
            &id,
            "--target",
            "file",
            "--path",
            &root.to_string_lossy(),
            "--action",
            "shared_read_write"
        ]
    )
    .status
    .success());
    assert_eq!(
        std::fs::read(store.containers_path()).unwrap(),
        policy_bytes
    );
    assert!(run(
        &root,
        &[
            "container",
            "policy",
            "add",
            &id,
            "--target",
            "registry",
            "--path",
            "HKCU\\Software\\FixtureVendor\\FixtureApp",
            "--action",
            "isolated_write"
        ]
    )
    .status
    .success());
    assert!(run(
        &root,
        &["container", "policy", "remove", &id, "--index", "4"]
    )
    .status
    .success());
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

#[cfg(windows)]
#[test]
#[ignore = "requires actual local fixed NTFS C and D volumes; run explicitly on the Windows fixture host"]
fn actual_c_and_d_cli_shared_exceptions_and_cow_restriction() {
    let root = std::env::temp_dir().join(format!("aura-cli-policy26-{}", uuid::Uuid::new_v4()));
    let application = root.with_extension("application");
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let shared = workspace
        .join("target")
        .join(format!("policy26-cli-shared-{}", uuid::Uuid::new_v4()));
    assert!(application
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("c:"));
    assert!(shared
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("d:"));
    std::fs::create_dir_all(&application).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
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
    for (path, action) in [
        (application.clone(), "isolated_write"),
        (shared.clone(), "shared_read_only"),
        (shared.join("rw"), "shared_read_write"),
        (shared.join("deny"), "deny"),
    ] {
        let added = run(
            &root,
            &[
                "container",
                "policy",
                "add",
                &id,
                "--target",
                "file",
                "--path",
                &path.to_string_lossy(),
                "--action",
                action,
            ],
        );
        assert!(
            added.status.success(),
            "{}",
            String::from_utf8_lossy(&added.stderr)
        );
    }
    let preview = run(
        &root,
        &[
            "container",
            "policy",
            "preview",
            &id,
            "--target",
            "file",
            "--path",
            &shared.join("rw/data").to_string_lossy(),
        ],
    );
    assert!(preview.status.success());
    let text = String::from_utf8(preview.stdout).unwrap();
    assert!(text.contains("host_write_exception\ttrue") && text.contains("can_authorize\tfalse"));
    let before = std::fs::read(store.containers_path()).unwrap();
    let failure = run(
        &root,
        &[
            "container",
            "policy",
            "add",
            &id,
            "--target",
            "file",
            "--path",
            &shared.join("second-cow").to_string_lossy(),
            "--action",
            "isolated_write",
        ],
    );
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("isolated-write"));
    assert_eq!(std::fs::read(store.containers_path()).unwrap(), before);
    std::fs::remove_dir_all(application).unwrap();
    std::fs::remove_dir_all(shared).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
