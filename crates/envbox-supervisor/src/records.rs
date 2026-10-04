//! Bounded, per-instance journal loading. A malformed A record never erases B.
use crate::{RunCommand, RunResult};
use envbox_core::RunSnapshot;
use envbox_storage::ConfigStore;
use std::io::{self, Read};
use std::path::Path;
use uuid::Uuid;

pub(crate) struct Record {
    pub command: RunCommand,
    pub result: RunResult,
    pub snapshot: Option<RunSnapshot>,
}
fn plain(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    if std::fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "journal reparse point is not followed",
        ));
    }
    Ok(())
}

pub(crate) fn scan(store: &ConfigStore) -> io::Result<Vec<Record>> {
    let directory = store.root().join("containers");
    if !directory.try_exists()? {
        return Ok(vec![]);
    }
    plain(&directory)?;
    let mut records = Vec::new();
    for (workspace_count, workspace) in std::fs::read_dir(&directory)?.enumerate() {
        if workspace_count >= 128 {
            return Err(io::Error::other(
                "ownership journal exceeds bounded workspace discovery capacity (128)",
            ));
        }
        let workspace = workspace?;
        let Ok(container_id) = workspace.file_name().to_string_lossy().parse::<Uuid>() else {
            continue;
        };
        if plain(&workspace.path()).is_err() {
            records.push(lost(
                container_id,
                Uuid::new_v4(),
                "workspace journal is a reparse point or unreadable",
            ));
            continue;
        }
        let runs = workspace.path().join("runs");
        match runs.try_exists() {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => {
                records.push(lost(
                    container_id,
                    Uuid::new_v4(),
                    &format!("cannot inspect Run journal: {error}"),
                ));
                continue;
            }
        }
        if plain(&runs).is_err() {
            records.push(lost(
                container_id,
                Uuid::new_v4(),
                "Run journal is a reparse point or unreadable",
            ));
            continue;
        }
        let entries = match std::fs::read_dir(&runs) {
            Ok(entries) => entries,
            Err(error) => {
                records.push(lost(
                    container_id,
                    Uuid::new_v4(),
                    &format!("cannot enumerate Run journal: {error}"),
                ));
                continue;
            }
        };
        for (entry_count, entry) in entries.enumerate() {
            if entry_count >= 128 {
                records.push(lost(
                    container_id,
                    Uuid::new_v4(),
                    "workspace ownership journal exceeds bounded discovery capacity (128 entries)",
                ));
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    records.push(lost(
                        container_id,
                        Uuid::new_v4(),
                        &format!("cannot inspect Run journal: {error}"),
                    ));
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(instance_id) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .and_then(|value| value.parse::<Uuid>().ok())
            else {
                continue;
            };
            let result = read(&path).and_then(|result| {
                if result.container_id != container_id || result.instance_id != instance_id {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "record identity does not match its UUID path",
                    ));
                }
                Ok(result)
            });
            match result {
                Err(error) => records.push(lost(
                    container_id,
                    instance_id,
                    &format!("invalid Run record: {error}"),
                )),
                Ok(mut result) => {
                    let command = RunCommand {
                        container_id,
                        instance_id,
                        application_id: result.application_id,
                    };
                    let snapshot = store.load_run_snapshot(container_id, instance_id);
                    let reason = match &snapshot {
                        Err(error) => Some(format!("immutable snapshot unavailable: {error}")),
                        Ok(_) if result.record_schema != 2 => {
                            Some("unsupported Run record schema; explicit upgrade required".into())
                        }
                        Ok(snapshot)
                            if snapshot.content_digest != result.snapshot_digest
                                || snapshot.configuration_id != result.configuration_id
                                || snapshot.effective_profile.id != result.profile_id =>
                        {
                            Some("record/snapshot digest or Profile identity mismatch".into())
                        }
                        Ok(_) => None,
                    };
                    let valid = reason.is_none();
                    if let Some(reason) = reason {
                        result.state = "TrackingLost".into();
                        result.error = Some(reason);
                    }
                    records.push(Record {
                        command,
                        result,
                        snapshot: if valid { snapshot.ok() } else { None },
                    });
                }
            }
        }
    }
    Ok(records)
}
fn read(path: &Path) -> io::Result<RunResult> {
    plain(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Run journal exceeds one MiB",
        ));
    }
    serde_json::from_slice(&bytes).map_err(Into::into)
}
fn lost(container_id: Uuid, instance_id: Uuid, reason: &str) -> Record {
    let command = RunCommand {
        container_id,
        instance_id,
        application_id: Uuid::nil(),
    };
    let (_, mut result) = crate::runs::failed(&command, reason);
    result.state = "TrackingLost".into();
    Record {
        command,
        result,
        snapshot: None,
    }
}
