use crate::{validate_profile, ConfigStore, StorageError};
use envbox_core::run_snapshot::MAX_RUN_SNAPSHOT_BYTES;
use envbox_core::{DomainError, RunSnapshot};
use std::io::{Read, Write};
use uuid::Uuid;

fn invalid(reason: impl Into<String>) -> StorageError {
    DomainError::InvalidContainer(format!("Run snapshot: {}", reason.into())).into()
}

impl ConfigStore {
    pub fn run_snapshot_path(&self, container_id: Uuid, instance_id: Uuid) -> std::path::PathBuf {
        self.root()
            .join("containers")
            .join(container_id.to_string())
            .join("snapshots")
            .join(format!("{instance_id}.toml"))
    }

    /// Configuration preparation only: this method starts no process and grants no IPC authority.
    pub fn prepare_run_snapshot(
        &self,
        container_id: Uuid,
        instance_id: Uuid,
    ) -> Result<RunSnapshot, StorageError> {
        let document = self.load_containers()?;
        let container = document
            .containers
            .iter()
            .find(|container| container.id == container_id)
            .ok_or_else(|| invalid("Container UUID not found"))?;
        let profiles = self.load_profiles()?;
        let profile = profiles
            .profiles
            .iter()
            .find(|profile| profile.id == container.profile_id)
            .ok_or_else(|| invalid(format!("Profile {} not found", container.profile_id)))?;
        validate_profile(profile)?;
        super::containers::validate_policy_volumes(&container.storage_policy)?;
        let snapshot = RunSnapshot::new(container, profile, instance_id)?;
        if self
            .run_snapshot_path(container_id, instance_id)
            .try_exists()?
        {
            let existing = self.load_run_snapshot(container_id, instance_id)?;
            if existing.configuration_id == snapshot.configuration_id {
                return Ok(existing);
            }
            return Err(invalid(
                "Instance snapshot already exists with different immutable configuration",
            ));
        }
        self.save_run_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    /// Atomically publish a complete file without replacing any existing Instance identity.
    pub fn save_run_snapshot(&self, snapshot: &RunSnapshot) -> Result<(), StorageError> {
        snapshot.validate()?;
        snapshot
            .storage_policy
            .validate(&self.root().to_string_lossy())?;
        let path = self.run_snapshot_path(snapshot.container_id, snapshot.instance_id);
        let text = toml::to_string_pretty(snapshot)?;
        if text.len() > MAX_RUN_SNAPSHOT_BYTES {
            return Err(invalid("encoded snapshot exceeds one MiB limit"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("missing snapshot directory"))?;
        std::fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".prepare-{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<(), StorageError> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            // Same-directory hard-link publication is atomic and never overwrites a destination.
            match std::fs::hard_link(&temporary, &path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if self.load_run_snapshot(snapshot.container_id, snapshot.instance_id)?
                        == *snapshot
                    {
                        Ok(())
                    } else {
                        Err(invalid(
                            "Instance snapshot already exists with different immutable data",
                        ))
                    }
                }
                Err(err) => Err(err.into()),
            }
        })();
        let _ = std::fs::remove_file(temporary);
        result
    }

    /// A snapshot can be inspected after its source Container/Profile has been removed.
    pub fn load_run_snapshot(
        &self,
        container_id: Uuid,
        instance_id: Uuid,
    ) -> Result<RunSnapshot, StorageError> {
        let path = self.run_snapshot_path(container_id, instance_id);
        let mut text = String::new();
        std::fs::File::open(path)?
            .take(MAX_RUN_SNAPSHOT_BYTES as u64 + 1)
            .read_to_string(&mut text)?;
        if text.len() > MAX_RUN_SNAPSHOT_BYTES {
            return Err(invalid("encoded snapshot exceeds one MiB limit"));
        }
        let value: toml::Value = toml::from_str(&text)?;
        let snapshot: RunSnapshot = value.try_into()?;
        snapshot.validate()?;
        snapshot
            .storage_policy
            .validate(&self.root().to_string_lossy())?;
        if snapshot.container_id != container_id
            || snapshot.instance_id != instance_id
            || snapshot.snapshot_id != instance_id
        {
            return Err(invalid(
                "requested Container/Instance does not match stored identity",
            ));
        }
        Ok(snapshot)
    }
}
