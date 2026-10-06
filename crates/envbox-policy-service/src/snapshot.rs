use crate::{bundle::InstalledBundle, protocol::LaunchRequest};
use envbox_core::RunSnapshot;
use envbox_storage::ConfigStore;
use std::path::{Path, PathBuf};
use windows::Win32::{
    Foundation::HANDLE,
    Security::{ImpersonateLoggedOnUser, RevertToSelf},
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY},
};
type Result<T> = std::result::Result<T, String>;
struct Revert;
impl Drop for Revert {
    fn drop(&mut self) {
        unsafe {
            if RevertToSelf().is_err() {
                std::process::abort();
            }
        }
    }
}

pub(crate) fn validate_reference(r: &LaunchRequest, snapshot: &RunSnapshot) -> Result<()> {
    snapshot.validate().map_err(|e| e.to_string())?;
    if (
        snapshot.container_id,
        snapshot.instance_id,
        snapshot.snapshot_id,
        snapshot.effective_profile.id,
    ) != (r.container_id, r.instance_id, r.snapshot_id, r.profile_id)
        || snapshot.configuration_id != r.configuration_id
        || snapshot.content_digest != r.content_digest
    {
        return Err("authoritative snapshot reference mismatch".into());
    }
    if snapshot.effective_profile.environment.iter().any(|(k, v)| {
        k.to_ascii_uppercase().starts_with("ENVBOX_")
            || k.to_ascii_uppercase().starts_with("AURA_")
            || v.contains('\0')
    }) {
        return Err("reserved effective snapshot environment".into());
    }
    Ok(())
}
pub(crate) fn load_for_token(token: HANDLE, r: &LaunchRequest) -> Result<RunSnapshot> {
    unsafe {
        ImpersonateLoggedOnUser(token).map_err(|e| e.to_string())?;
    }
    let _revert = Revert;
    let root = user_root(token)?;
    load_from_root(&root, r)
}
fn user_root(token: HANDLE) -> Result<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, token)
            .map_err(|e| e.to_string())?;
        let decoded = path.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(path.0.cast()));
        let root = PathBuf::from(decoded?).join(envbox_storage::DATA_DIR_NAME);
        crate::protocol::validate_local_path(&root)?;
        Ok(root)
    }
}
fn load_from_root(root: &Path, r: &LaunchRequest) -> Result<RunSnapshot> {
    // Caller holds user impersonation throughout KnownFolder, lease walk,
    // storage reads and validation. No System default/environment root.
    let store = ConfigStore::new(root);
    let paths = [
        store.containers_path(),
        store.profiles_path(),
        store.run_snapshot_path(r.container_id, r.instance_id),
    ];
    let mut leases = vec![];
    for path in &paths {
        leases.extend(InstalledBundle::lease_input_path(path, false)?);
        if std::fs::metadata(path).map_err(|e| e.to_string())?.len()
            > envbox_core::run_snapshot::MAX_RUN_SNAPSHOT_BYTES as u64
        {
            return Err("authoritative document exceeds one MiB".into());
        }
    }
    let containers = store.load_containers().map_err(|e| e.to_string())?;
    let container = containers
        .containers
        .iter()
        .find(|c| c.id == r.container_id)
        .ok_or("requested Container does not exist")?;
    if container.profile_id != r.profile_id {
        return Err("Container/Profile binding mismatch".into());
    }
    let profiles = store.load_profiles().map_err(|e| e.to_string())?;
    if !profiles.profiles.iter().any(|p| p.id == r.profile_id) {
        return Err("bound Profile does not exist".into());
    }
    let snapshot = store
        .load_run_snapshot(r.container_id, r.instance_id)
        .map_err(|e| e.to_string())?;
    validate_reference(r, &snapshot)?;
    // Deliberately do not compare current live Profile contents: stored
    // effective Profile/configuration are immutable for this instance.
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_launcher::launcher::win::SafeHandle;
    use windows::Win32::{
        Security::{TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    struct Fixture {
        root: PathBuf,
        store: ConfigStore,
        request: LaunchRequest,
        snapshot: RunSnapshot,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!("service-snapshot-{}", uuid::Uuid::new_v4()));
            let store = ConfigStore::new(&root);
            let (request, snapshot, container) = crate::protocol::tests::fixture();
            store
                .save_profiles(&envbox_storage::ProfileDocument {
                    profiles: vec![snapshot.effective_profile.clone()],
                })
                .unwrap();
            store
                .save_containers(&envbox_storage::ContainerDocument {
                    schema_version: 1,
                    containers: vec![container],
                })
                .unwrap();
            store.save_run_snapshot(&snapshot).unwrap();
            Self {
                root,
                store,
                request,
                snapshot,
            }
        }
        fn read_as_current_user(&self) -> Result<RunSnapshot> {
            unsafe {
                let mut handle = HANDLE::default();
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE,
                    &mut handle,
                )
                .map_err(|e| e.to_string())?;
                let token = SafeHandle(handle);
                ImpersonateLoggedOnUser(token.0).map_err(|e| e.to_string())?;
                let _revert = Revert;
                load_from_root(&self.root, &self.request)
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let allowed = self
                .root
                .parent()
                .and_then(|p| std::fs::canonicalize(p).ok());
            let resolved = std::fs::canonicalize(&self.root).ok();
            if resolved.as_ref().and_then(|p| p.parent()) == allowed.as_deref()
                && self
                    .root
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("service-snapshot-"))
            {
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
    }
    #[test]
    fn native_snapshot_read_and_live_profile_edits_preserve_frozen_content() {
        let f = Fixture::new();
        assert_eq!(f.read_as_current_user().unwrap(), f.snapshot);
        let mut changed = f.snapshot.effective_profile.clone();
        changed.name = "edited live profile".into();
        changed
            .environment
            .insert("AFTER_PREPARATION".into(), "new".into());
        f.store
            .save_profiles(&envbox_storage::ProfileDocument {
                profiles: vec![changed],
            })
            .unwrap();
        let read = f.read_as_current_user().unwrap();
        assert_eq!(read, f.snapshot);
        assert!(!read
            .effective_profile
            .environment
            .contains_key("AFTER_PREPARATION"));
    }
    #[test]
    fn missing_or_tampered_snapshot_fails_native_read() {
        let f = Fixture::new();
        let path = f
            .store
            .run_snapshot_path(f.request.container_id, f.request.instance_id);
        std::fs::remove_file(&path).unwrap();
        assert!(f.read_as_current_user().is_err());
        f.store.save_run_snapshot(&f.snapshot).unwrap();
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&f.snapshot.content_digest, &"0".repeat(64));
        std::fs::write(path, text).unwrap();
        assert!(f.read_as_current_user().is_err());
    }
    #[test]
    fn wrong_container_profile_and_client_digest_fail_native_read() {
        let mut f = Fixture::new();
        f.request.configuration_id = "0".repeat(64);
        assert!(f.read_as_current_user().is_err());
        f.request.configuration_id = f.snapshot.configuration_id.clone();
        f.request.profile_id = uuid::Uuid::new_v4();
        assert!(f.read_as_current_user().is_err());
        f.request.profile_id = f.snapshot.effective_profile.id;
        let mut doc = f.store.load_containers().unwrap();
        doc.containers.clear();
        f.store.save_containers(&doc).unwrap();
        assert!(f.read_as_current_user().is_err());
    }
    #[test]
    fn known_folder_root_uses_token_without_configuration_environment_override() {
        struct EnvRestore(Option<std::ffi::OsString>);
        impl Drop for EnvRestore {
            fn drop(&mut self) {
                if let Some(value) = &self.0 {
                    std::env::set_var("ENVBOX_CONFIG_ROOT", value);
                } else {
                    std::env::remove_var("ENVBOX_CONFIG_ROOT");
                }
            }
        }
        let _restore = EnvRestore(std::env::var_os("ENVBOX_CONFIG_ROOT"));
        let fake = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("forbidden-configuration-environment-root");
        std::env::set_var("ENVBOX_CONFIG_ROOT", &fake);
        unsafe {
            let mut handle = HANDLE::default();
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut handle,
            )
            .unwrap();
            let token = SafeHandle(handle);
            let root = user_root(token.0).unwrap();
            assert!(root.is_absolute());
            assert_eq!(root.file_name().unwrap(), envbox_storage::DATA_DIR_NAME);
            assert_ne!(root, fake);
            // This helper reads KnownFolder only; no ConfigStore default or
            // inherited LOCALAPPDATA/ENVBOX_CONFIG_ROOT enters its derivation.
        }
    }
}
