use crate::{ConfigStore, StorageError};
use envbox_core::{Container, DomainError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerDocument {
    pub schema_version: u32,
    pub containers: Vec<Container>,
}

impl Default for ContainerDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            containers: Vec::new(),
        }
    }
}

impl ConfigStore {
    pub fn containers_path(&self) -> std::path::PathBuf {
        self.root().join("containers.toml")
    }

    pub fn load_containers(&self) -> Result<ContainerDocument, StorageError> {
        let path = self.containers_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ContainerDocument::default())
            }
            Err(err) => return Err(err.into()),
        };
        let doc: ContainerDocument = toml::from_str(&text)?;
        self.validate_containers(&doc)?;
        Ok(doc)
    }

    fn validate_containers(&self, doc: &ContainerDocument) -> Result<(), StorageError> {
        if doc.schema_version != 1 {
            return Err(DomainError::InvalidContainer(format!(
                "unsupported schema {}",
                doc.schema_version
            ))
            .into());
        }
        let mut ids = HashSet::new();
        for workspace in &doc.containers {
            workspace.validate()?;
            workspace
                .storage_policy
                .validate(&self.root().to_string_lossy())?;
            if !ids.insert(workspace.id) {
                return Err(DomainError::InvalidContainer(format!(
                    "duplicate UUID {}",
                    workspace.id
                ))
                .into());
            }
        }
        Ok(())
    }

    pub fn save_containers(&self, doc: &ContainerDocument) -> Result<(), StorageError> {
        self.validate_containers(doc)?;
        let existing = self.load_containers()?;
        let profiles = self.load_profiles()?;
        for workspace in &doc.containers {
            validate_policy_volumes(&workspace.storage_policy)?;
            if !profiles
                .profiles
                .iter()
                .any(|profile| profile.id == workspace.profile_id)
                && !existing.containers.iter().any(|previous| {
                    previous.id == workspace.id && previous.profile_id == workspace.profile_id
                })
            {
                return Err(DomainError::InvalidContainer(format!(
                    "Profile {} not found for {}",
                    workspace.profile_id, workspace.id
                ))
                .into());
            }
        }
        // A newer or corrupt on-disk document must not be replaced by an older client.
        match std::fs::read_to_string(self.containers_path()) {
            Ok(text) => {
                let existing: ContainerDocument = toml::from_str(&text)?;
                if existing.schema_version != 1 {
                    return Err(DomainError::InvalidContainer(format!(
                        "unsupported existing schema {}",
                        existing.schema_version
                    ))
                    .into());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        std::fs::create_dir_all(self.root())?;
        let path = self.containers_path();
        let temporary = self
            .root()
            .join(format!(".containers-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<(), StorageError> {
            let text = toml::to_string_pretty(doc)?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            atomic_replace(&temporary, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

pub(super) fn validate_policy_volumes(
    policy: &envbox_core::storage_policy::StoragePolicy,
) -> Result<(), StorageError> {
    use envbox_core::storage_policy::{normalize_storage_path, StorageAction, StorageTarget};
    let mut drive = None;
    for rule in &policy.rules {
        if rule.target != StorageTarget::FileDirectory {
            continue;
        }
        let normalized = normalize_storage_path(rule.target, &rule.path)?;
        let current = &normalized[..2];
        if rule.action == StorageAction::IsolatedWrite
            && drive.as_ref().is_some_and(|previous| previous != current)
        {
            return Err(DomainError::InvalidContainer(
                "storage policy: cross-volume isolated-write scopes are unsupported".into(),
            )
            .into());
        }
        if rule.action == StorageAction::IsolatedWrite {
            drive = Some(current.to_owned());
        }
        #[cfg(windows)]
        {
            use windows::core::PCWSTR;
            use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};
            let root: Vec<u16> = format!("{current}\\")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != 3 {
                return Err(DomainError::InvalidContainer(
                    "storage policy: only local fixed NTFS volumes are supported".into(),
                )
                .into());
            }
            let mut filesystem = [0u16; 64];
            unsafe {
                GetVolumeInformationW(
                    PCWSTR(root.as_ptr()),
                    None,
                    None,
                    None,
                    None,
                    Some(&mut filesystem),
                )
            }
            .map_err(|err| {
                DomainError::InvalidContainer(format!(
                    "storage policy: volume qualification failed: {err}"
                ))
            })?;
            let end = filesystem
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(filesystem.len());
            if !String::from_utf16_lossy(&filesystem[..end]).eq_ignore_ascii_case("NTFS") {
                return Err(DomainError::InvalidContainer(
                    "storage policy: volume is not NTFS".into(),
                )
                .into());
            }
        }
        #[cfg(not(windows))]
        return Err(DomainError::InvalidContainer(
            "storage policy: NTFS qualification requires Windows".into(),
        )
        .into());
    }
    Ok(())
}

pub(super) fn atomic_replace(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let src: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let dst: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            MoveFileExW(
                PCWSTR(src.as_ptr()),
                PCWSTR(dst.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|err| std::io::Error::from_raw_os_error(err.code().0 & 0xffff))
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(source, destination)
    }
}
