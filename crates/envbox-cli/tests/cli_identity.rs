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
fn identity_create_update_clear_and_invalid_edit_preserve_store() {
    let root = std::env::temp_dir().join(format!("aura-cli-identity-{}", uuid::Uuid::new_v4()));
    let created = run(
        &root,
        &[
            "profile",
            "add",
            "--name",
            "identity",
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
            "--computer-name",
            "AURA-A",
            "--user-name",
            "profile_a",
            "--mac-address",
            "02:ab:cd:00:00:01",
            "--machine-guid",
            "4B7E08F5-BA3E-4573-81F7-76548F934051",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let id = String::from_utf8(created.stdout).unwrap().trim().to_owned();
    let store = envbox_storage::ConfigStore::new(&root);
    let profile = store.load_profiles().unwrap().profiles.remove(0);
    assert_eq!(profile.identity.computer_name.as_deref(), Some("AURA-A"));
    assert_eq!(
        profile.identity.mac_address.as_deref(),
        Some("02:AB:CD:00:00:01")
    );
    assert_eq!(
        profile.identity.machine_guid.as_deref(),
        Some("4b7e08f5-ba3e-4573-81f7-76548f934051")
    );
    let shown = run(&root, &["profile", "identity", "show", &id]);
    assert!(shown.status.success());
    assert!(String::from_utf8(shown.stdout)
        .unwrap()
        .contains("computer_name=AURA-A"));
    let before = std::fs::read(store.profiles_path()).unwrap();
    for args in [
        vec![
            "profile",
            "identity",
            "set",
            &id,
            "--mac-address",
            "01:00:00:00:00:01",
        ],
        vec![
            "profile",
            "identity",
            "set",
            &id,
            "--user-name",
            "a",
            "--clear",
            "user_name",
        ],
        vec!["profile", "identity", "set", &id, "--machine-guid"],
        vec!["profile", "identity", "reset", &id, "unexpected"],
    ] {
        assert!(!run(&root, &args).status.success());
        assert_eq!(before, std::fs::read(store.profiles_path()).unwrap());
    }
    assert!(run(
        &root,
        &[
            "profile",
            "identity",
            "set",
            &id,
            "--computer-name",
            "AURA-B",
            "--clear",
            "user_name"
        ]
    )
    .status
    .success());
    let profile = store.load_profiles().unwrap().profiles.remove(0);
    assert_eq!(profile.identity.computer_name.as_deref(), Some("AURA-B"));
    assert!(profile.identity.user_name.is_none());
    assert!(profile.identity.machine_guid.is_some());
    assert!(run(&root, &["profile", "identity", "reset", &id])
        .status
        .success());
    assert!(store.load_profiles().unwrap().profiles[0]
        .identity
        .is_host());
    assert!(
        String::from_utf8(run(&root, &["profile", "identity", "show", &id]).stdout)
            .unwrap()
            .contains("machine_guid=(host)")
    );
}
