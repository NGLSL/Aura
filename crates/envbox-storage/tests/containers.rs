use envbox_core::{Container, ContainerMode};
use envbox_storage::{ConfigStore, ContainerDocument};

fn fixture() -> (ConfigStore, std::path::PathBuf) {
    use envbox_core::{DnsMode, DnsProfile, EnvironmentProfile, LocaleProfile, TimezoneProfile};
    let root = std::env::temp_dir().join(format!("aura-container-{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(&root);
    store
        .save_profiles(&envbox_storage::ProfileDocument {
            profiles: vec![EnvironmentProfile {
                id: uuid::Uuid::from_u128(1),
                name: "US".into(),
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
                environment: Default::default(),
                registry: Default::default(),
                browser: Default::default(),
            }],
        })
        .unwrap();
    std::fs::write(store.applications_path(), "applications = []\n").unwrap();
    (store, root)
}

#[test]
fn restart_roundtrip_edit_preserves_identity_and_legacy_documents() {
    let (store, root) = fixture();
    let old_profiles = std::fs::read(store.profiles_path()).unwrap();
    let old_apps = std::fs::read(store.applications_path()).unwrap();
    let a = Container::new("same name", uuid::Uuid::from_u128(1));
    let b = Container::new("same name", uuid::Uuid::from_u128(1));
    assert_ne!(a.id, b.id);
    let original = ContainerDocument {
        schema_version: 1,
        containers: vec![a, b],
    };
    store.save_containers(&original).unwrap();
    let restarted = ConfigStore::new(&root);
    assert_eq!(restarted.load_containers().unwrap(), original);
    let mut edited = original.clone();
    edited.containers[0].name = "renamed".into();
    restarted.save_containers(&edited).unwrap();
    assert_eq!(store.load_containers().unwrap(), edited);
    assert_eq!(std::fs::read(store.profiles_path()).unwrap(), old_profiles);
    assert_eq!(std::fs::read(store.applications_path()).unwrap(), old_apps);
    let bytes = std::fs::read(store.containers_path()).unwrap();
    edited.containers[0].profile_id = uuid::Uuid::new_v4();
    assert!(store.save_containers(&edited).is_err());
    assert_eq!(std::fs::read(store.containers_path()).unwrap(), bytes);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepare_run_snapshot_is_immutable_and_survives_profile_deletion() {
    let (store, root) = fixture();
    let workspace = Container::new("A", uuid::Uuid::from_u128(1));
    store
        .save_containers(&ContainerDocument {
            schema_version: 1,
            containers: vec![workspace.clone()],
        })
        .unwrap();
    let instance = uuid::Uuid::new_v4();
    let frozen = store.prepare_run_snapshot(workspace.id, instance).unwrap();
    assert_eq!(
        store.prepare_run_snapshot(workspace.id, instance).unwrap(),
        frozen
    );
    let before = std::fs::read(store.run_snapshot_path(workspace.id, instance)).unwrap();
    let mut profiles = store.load_profiles().unwrap();
    profiles.profiles[0].name = "changed".into();
    store.save_profiles(&profiles).unwrap();
    assert_eq!(
        store.load_run_snapshot(workspace.id, instance).unwrap(),
        frozen
    );
    assert_eq!(
        std::fs::read(store.run_snapshot_path(workspace.id, instance)).unwrap(),
        before
    );
    assert_ne!(
        store
            .prepare_run_snapshot(workspace.id, uuid::Uuid::new_v4())
            .unwrap()
            .configuration_id,
        frozen.configuration_id
    );
    assert!(store.prepare_run_snapshot(workspace.id, instance).is_err());
    store.save_run_snapshot(&frozen).unwrap();
    std::fs::write(store.profiles_path(), "profiles = []\n").unwrap();
    assert_eq!(
        store.load_containers().unwrap().containers[0].id,
        workspace.id
    );
    assert_eq!(
        store.load_run_snapshot(workspace.id, instance).unwrap(),
        frozen
    );
    assert!(store
        .prepare_run_snapshot(workspace.id, uuid::Uuid::new_v4())
        .is_err());
    assert_eq!(
        std::fs::read(store.run_snapshot_path(workspace.id, instance)).unwrap(),
        before
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn original_v1_snapshot_digest_and_bytes_survive_typed_dns_migration() {
    let root = std::env::temp_dir().join(format!("aura-v1-snapshot-{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(&root);
    let wire = include_str!("fixtures/run-snapshot-v1.toml");
    let snapshot: envbox_core::RunSnapshot = toml::from_str(wire).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.profile_schema_version, 1);
    assert_eq!(
        snapshot.configuration_id,
        "888b0b6b52090cd2ad6c625fbb520e319dce0544d83f0fb6d7794bc3517c7527"
    );
    assert_eq!(
        snapshot.content_digest,
        "85ca1fc3325df45b5a9caee9fef58e96ae6379e82fe97b417fa242703e05f4c7"
    );
    let path = store.run_snapshot_path(snapshot.container_id, snapshot.instance_id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, wire).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        store
            .load_run_snapshot(snapshot.container_id, snapshot.instance_id)
            .unwrap(),
        snapshot
    );
    store.save_run_snapshot(&snapshot).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(snapshot.effective_profile.dns.upstreams.len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_a_b_binding_profile_switch_and_integrity_failures_are_explicit() {
    let (store, root) = fixture();
    let mut profiles = store.load_profiles().unwrap();
    let mut b_profile = profiles.profiles[0].clone();
    b_profile.id = uuid::Uuid::from_u128(2);
    b_profile.name = "B Profile".into();
    b_profile
        .environment
        .insert("SNAPSHOT_TEST".into(), "B".into());
    profiles.profiles.push(b_profile);
    store.save_profiles(&profiles).unwrap();
    let a = Container::new("same", uuid::Uuid::from_u128(1));
    let b = Container::new("same", uuid::Uuid::from_u128(2));
    let mut doc = ContainerDocument {
        schema_version: 1,
        containers: vec![a.clone(), b.clone()],
    };
    store.save_containers(&doc).unwrap();
    let instance = uuid::Uuid::new_v4();
    let old = store.prepare_run_snapshot(a.id, instance).unwrap();
    let b_snapshot = store
        .prepare_run_snapshot(b.id, uuid::Uuid::new_v4())
        .unwrap();
    assert_ne!(old.configuration_id, b_snapshot.configuration_id);
    assert!(store.load_run_snapshot(b.id, instance).is_err());
    doc.containers[0].profile_id = b.profile_id;
    store.save_containers(&doc).unwrap();
    assert_eq!(store.load_run_snapshot(a.id, instance).unwrap(), old);
    let new = store
        .prepare_run_snapshot(a.id, uuid::Uuid::new_v4())
        .unwrap();
    assert_eq!(new.effective_profile.id, b.profile_id);
    assert_eq!(new.configuration_id, b_snapshot.configuration_id);
    let path = store.run_snapshot_path(a.id, instance);
    let original = std::fs::read_to_string(&path).unwrap();
    let mut tampered = old.clone();
    tampered.configuration_id = "0".repeat(64);
    assert!(store.save_run_snapshot(&tampered).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    for mutation in 0..5 {
        let mut value: toml::Value = toml::from_str(&original).unwrap();
        match mutation {
            0 => {
                value["configuration_id"] = toml::Value::String("0".repeat(64));
            }
            1 => {
                value["effective_profile"]
                    .as_table_mut()
                    .unwrap()
                    .remove("browser");
            }
            2 => {
                value["effective_profile"]["locale"]
                    .as_table_mut()
                    .unwrap()
                    .insert("unknown".into(), toml::Value::String("extra".into()));
            }
            3 => {
                value["schema_version"] = toml::Value::Integer(99);
            }
            _ => {
                value
                    .as_table_mut()
                    .unwrap()
                    .insert("unknown".into(), toml::Value::Boolean(true));
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&value).unwrap()).unwrap();
        assert!(
            store.load_run_snapshot(a.id, instance).is_err(),
            "mutation {mutation}"
        );
    }
    std::fs::write(&path, toml::to_string_pretty(&b_snapshot).unwrap()).unwrap();
    assert!(store
        .load_run_snapshot(a.id, instance)
        .unwrap_err()
        .to_string()
        .contains("identity"));
    std::fs::write(
        &path,
        "x".repeat(envbox_core::run_snapshot::MAX_RUN_SNAPSHOT_BYTES + 1),
    )
    .unwrap();
    assert!(store
        .load_run_snapshot(a.id, instance)
        .unwrap_err()
        .to_string()
        .contains("limit"));
    std::fs::write(&path, &original).unwrap();
    profiles.profiles[1].environment.insert(
        "OVERSIZED".into(),
        "x".repeat(envbox_core::run_snapshot::MAX_RUN_SNAPSHOT_BYTES),
    );
    store.save_profiles(&profiles).unwrap();
    assert!(store
        .prepare_run_snapshot(a.id, uuid::Uuid::new_v4())
        .is_err());
    assert_eq!(store.load_run_snapshot(a.id, instance).unwrap(), old);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_profile_metadata_does_not_break_other_workspaces_or_allow_new_bad_reference() {
    let (store, root) = fixture();
    let a = Container::new("A", uuid::Uuid::from_u128(1));
    let b = Container::new("B", uuid::Uuid::from_u128(1));
    let mut doc = ContainerDocument {
        schema_version: 1,
        containers: vec![a.clone(), b.clone()],
    };
    store.save_containers(&doc).unwrap();
    let mut profiles = store.load_profiles().unwrap();
    profiles.profiles[0].id = uuid::Uuid::from_u128(2);
    store.save_profiles(&profiles).unwrap();
    let loaded = store.load_containers().unwrap();
    assert_eq!(loaded.containers.len(), 2);
    doc.containers[1].profile_id = uuid::Uuid::from_u128(2);
    doc.containers[1].name = "B repaired".into();
    store.save_containers(&doc).unwrap();
    assert!(store
        .prepare_run_snapshot(a.id, uuid::Uuid::new_v4())
        .is_err());
    assert_eq!(
        store
            .prepare_run_snapshot(b.id, uuid::Uuid::new_v4())
            .unwrap()
            .effective_profile
            .id,
        uuid::Uuid::from_u128(2)
    );
    let before = std::fs::read(store.containers_path()).unwrap();
    doc.containers
        .push(Container::new("new invalid", uuid::Uuid::new_v4()));
    assert!(store.save_containers(&doc).is_err());
    assert_eq!(std::fs::read(store.containers_path()).unwrap(), before);
    std::fs::write(store.profiles_path(), "profiles = malformed").unwrap();
    assert_eq!(store.load_containers().unwrap().containers.len(), 2);
    assert!(store
        .prepare_run_snapshot(b.id, uuid::Uuid::new_v4())
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_container_schema_defaults_empty_storage_and_policy_errors_keep_bytes() {
    use envbox_core::storage_policy::{StorageAction, StorageRule, StorageTarget};
    let (store, root) = fixture();
    let id = uuid::Uuid::new_v4();
    let legacy = format!("schema_version = 1\n[[containers]]\nid = '{id}'\nname = 'old'\nprofile_id = '00000000-0000-0000-0000-000000000001'\nmode = 'compatibility'\ncreated_at_unix_ms = 123\n");
    std::fs::write(store.containers_path(), &legacy).unwrap();
    let mut doc = store.load_containers().unwrap();
    assert!(doc.containers[0].storage_policy.rules.is_empty());
    assert_eq!(doc.containers[0].mode, ContainerMode::Compatibility);
    let before = std::fs::read(store.containers_path()).unwrap();
    doc.containers[0].storage_policy.rules.push(StorageRule {
        target: StorageTarget::RegistrySubtree,
        path: "HKCU\\Software\\FixtureVendor\\App".into(),
        action: StorageAction::SharedReadOnly,
    });
    let frozen = doc.containers[0].storage_policy.clone();
    store.save_containers(&doc).unwrap();
    assert_eq!(store.load_containers().unwrap(), doc);
    let bytes = std::fs::read(store.containers_path()).unwrap();
    doc.containers[0].storage_policy.rules[0].action = StorageAction::Deny;
    assert_eq!(frozen.rules[0].action, StorageAction::SharedReadOnly);
    doc.containers[0].storage_policy.schema_version = 99;
    assert!(store.save_containers(&doc).is_err());
    assert_eq!(std::fs::read(store.containers_path()).unwrap(), bytes);
    assert_ne!(before, bytes);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "requires actual local fixed NTFS C and D volumes; run explicitly on the Windows fixture host"]
fn actual_c_and_d_shared_rules_do_not_inherit_cow_volume_restriction() {
    use envbox_core::storage_policy::{StorageAction, StorageRule, StorageTarget};
    let (store, root) = fixture();
    let application = root.with_extension("application");
    assert!(application
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("c:"));
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let shared = workspace
        .join("target")
        .join(format!("policy26-shared-{}", uuid::Uuid::new_v4()));
    assert!(shared
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("d:"));
    std::fs::create_dir_all(&application).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    let mut container = Container::new("A", uuid::Uuid::from_u128(1));
    for (path, action) in [
        (application.clone(), StorageAction::IsolatedWrite),
        (shared.clone(), StorageAction::SharedReadOnly),
        (shared.join("writable"), StorageAction::SharedReadWrite),
        (shared.join("denied"), StorageAction::Deny),
    ] {
        container.storage_policy.rules.push(StorageRule {
            target: StorageTarget::FileDirectory,
            path: path.to_string_lossy().into_owned(),
            action,
        });
    }
    let mut doc = ContainerDocument {
        schema_version: 1,
        containers: vec![container],
    };
    store.save_containers(&doc).unwrap();
    assert_eq!(store.load_containers().unwrap(), doc);
    let before = std::fs::read(store.containers_path()).unwrap();
    doc.containers[0].storage_policy.rules.push(StorageRule {
        target: StorageTarget::FileDirectory,
        path: shared.join("second-cow").to_string_lossy().into_owned(),
        action: StorageAction::IsolatedWrite,
    });
    assert!(store
        .save_containers(&doc)
        .unwrap_err()
        .to_string()
        .contains("isolated-write"));
    assert_eq!(std::fs::read(store.containers_path()).unwrap(), before);
    std::fs::remove_dir_all(application).unwrap();
    std::fs::remove_dir_all(shared).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_duplicate_metadata_fail_and_missing_profile_fails_prepare() {
    let (store, root) = fixture();
    let a = Container::new("A", uuid::Uuid::from_u128(1));
    let mut doc = ContainerDocument {
        schema_version: 1,
        containers: vec![a.clone(), a],
    };
    assert!(store
        .save_containers(&doc)
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    doc.containers.pop();
    store.save_containers(&doc).unwrap();
    let unknown_schema = "schema_version = 99\ncontainers = []\n";
    std::fs::write(store.containers_path(), unknown_schema).unwrap();
    assert!(store
        .save_containers(&doc)
        .unwrap_err()
        .to_string()
        .contains("schema"));
    assert_eq!(
        std::fs::read_to_string(store.containers_path()).unwrap(),
        unknown_schema
    );
    std::fs::remove_file(store.containers_path()).unwrap();
    store.save_containers(&doc).unwrap();
    std::fs::write(store.profiles_path(), "profiles = []\n").unwrap();
    let metadata = store.load_containers().unwrap();
    assert!(store
        .prepare_run_snapshot(metadata.containers[0].id, uuid::Uuid::new_v4())
        .unwrap_err()
        .to_string()
        .contains("Profile"));
    std::fs::write(store.containers_path(), "[invalid\n").unwrap();
    assert!(store.load_containers().is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn atomic_replace_failure_preserves_old_bytes_and_cleans_temporary() {
    use std::os::windows::fs::OpenOptionsExt;
    let (store, root) = fixture();
    let mut doc = ContainerDocument {
        schema_version: 1,
        containers: vec![Container::new("A", uuid::Uuid::from_u128(1))],
    };
    store.save_containers(&doc).unwrap();
    let before = std::fs::read(store.containers_path()).unwrap();
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(store.containers_path())
        .unwrap();
    doc.containers[0].name = "B".into();
    assert!(store.save_containers(&doc).is_err());
    assert_eq!(std::fs::read(store.containers_path()).unwrap(), before);
    assert!(!std::fs::read_dir(&root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
    drop(locked);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn absent_document_is_empty_and_invalid_reference_never_writes() {
    let root = std::env::temp_dir().join(format!("aura-container-{}", uuid::Uuid::new_v4()));
    let store = ConfigStore::new(&root);
    assert!(store.load_containers().unwrap().containers.is_empty());
    let workspace = Container::new("same name", uuid::Uuid::new_v4());
    assert!(store
        .save_containers(&ContainerDocument {
            schema_version: 1,
            containers: vec![workspace]
        })
        .is_err());
    assert!(!store.containers_path().exists());
}

#[test]
fn unsupported_mode_and_schema_fail_explicitly() {
    let root = std::env::temp_dir().join(format!("aura-container-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = ConfigStore::new(&root);
    std::fs::write(
        store.containers_path(),
        "schema_version = 99\ncontainers = []\n",
    )
    .unwrap();
    assert!(store
        .load_containers()
        .unwrap_err()
        .to_string()
        .contains("schema"));
    let mut workspace = Container::new("A", uuid::Uuid::new_v4());
    workspace.mode = ContainerMode::Container;
    assert!(workspace
        .validate()
        .unwrap_err()
        .to_string()
        .contains("unsupported"));
    std::fs::remove_dir_all(root).unwrap();
}
