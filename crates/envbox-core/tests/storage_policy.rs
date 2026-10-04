use envbox_core::storage_policy::{StorageAction, StoragePolicy, StorageRule, StorageTarget};

#[test]
fn lexical_alias_conflict_and_most_specific_preview_fail_closed() {
    let mut policy = StoragePolicy::default();
    policy.rules.push(StorageRule {
        target: StorageTarget::FileDirectory,
        path: "C:\\Users\\fixture\\AppData\\Local\\App".into(),
        action: StorageAction::IsolatedWrite,
    });
    policy.rules.push(StorageRule {
        target: StorageTarget::FileDirectory,
        path: "c:/users/fixture/appdata/local/app/cache".into(),
        action: StorageAction::SharedReadOnly,
    });
    let preview = policy
        .preview(
            StorageTarget::FileDirectory,
            "C:\\Users\\fixture\\AppData\\Local\\App\\Cache\\data",
            "D:\\AuraMetadata",
        )
        .unwrap();
    assert_eq!(
        preview.configured_action,
        Some(StorageAction::SharedReadOnly)
    );
    assert!(!preview.can_authorize);
    policy.rules.push(StorageRule {
        target: StorageTarget::FileDirectory,
        path: "C:/Users/fixture/AppData/Local/App/".into(),
        action: StorageAction::SharedReadWrite,
    });
    assert!(policy.validate("D:\\AuraMetadata").is_err());
}

#[test]
fn management_roots_and_unsupported_names_never_authorize() {
    for path in [
        "\\\\server\\share\\app",
        "C:\\",
        "C:\\Apps\\..\\Windows",
        "C:\\Apps\\CON",
        "C:\\Apps\\file:stream",
        "C:\\Apps\\trailing.",
    ] {
        let policy = StoragePolicy {
            schema_version: 1,
            rules: vec![StorageRule {
                target: StorageTarget::FileDirectory,
                path: path.into(),
                action: StorageAction::SharedReadWrite,
            }],
        };
        assert!(policy.validate("D:\\AuraMetadata").is_err(), "{path}");
    }
    let policy = StoragePolicy {
        schema_version: 1,
        rules: vec![StorageRule {
            target: StorageTarget::FileDirectory,
            path: "D:\\AuraMetadata\\containers\\same-name".into(),
            action: StorageAction::SharedReadWrite,
        }],
    };
    assert!(policy.validate("D:\\AuraMetadata").is_err());
    let preview = StoragePolicy::default()
        .preview(
            StorageTarget::FileDirectory,
            "C:\\Apps\\Unknown",
            "D:\\AuraMetadata",
        )
        .unwrap();
    assert_eq!(preview.configured_action, None);
    assert!(!preview.can_authorize);
}

#[test]
fn registry_scope_and_component_boundaries_are_explicit() {
    for path in [
        "HKLM\\Software\\Vendor\\App",
        "HKCU\\Software",
        "HKCU\\Software\\Classes\\App",
        "HKCU\\Software\\Aura\\Containers",
    ] {
        let policy = StoragePolicy {
            schema_version: 1,
            rules: vec![StorageRule {
                target: StorageTarget::RegistrySubtree,
                path: path.into(),
                action: StorageAction::SharedReadWrite,
            }],
        };
        assert!(policy.validate("D:\\AuraMetadata").is_err());
    }
    let policy = StoragePolicy {
        schema_version: 1,
        rules: vec![StorageRule {
            target: StorageTarget::RegistrySubtree,
            path: "HKCU\\Software\\Vendor\\App".into(),
            action: StorageAction::Deny,
        }],
    };
    assert_eq!(
        policy
            .preview(
                StorageTarget::RegistrySubtree,
                "hkcu/software/vendor/app/settings",
                "D:\\AuraMetadata"
            )
            .unwrap()
            .configured_action,
        Some(StorageAction::Deny)
    );
    assert_eq!(
        policy
            .preview(
                StorageTarget::RegistrySubtree,
                "hkcu/software/vendor/application",
                "D:\\AuraMetadata"
            )
            .unwrap()
            .configured_action,
        None
    );
}
