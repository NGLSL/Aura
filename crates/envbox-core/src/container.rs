//! A persistent workspace identity, independent of any one runtime session.
use crate::DomainError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerMode {
    #[default]
    Compatibility,
    Container,
    Strong,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    pub id: Uuid,
    pub name: String,
    pub profile_id: Uuid,
    pub mode: ContainerMode,
    pub created_at_unix_ms: u64,
    #[serde(default)]
    pub storage_policy: crate::storage_policy::StoragePolicy,
}

impl Container {
    pub fn new(name: impl Into<String>, profile_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            profile_id,
            mode: ContainerMode::Compatibility,
            storage_policy: Default::default(),
            created_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || self.name.chars().any(char::is_control)
        {
            return Err(DomainError::InvalidContainer(
                "identity and printable name are required".into(),
            ));
        }
        if self.mode != ContainerMode::Compatibility {
            return Err(DomainError::InvalidContainer(format!(
                "unsupported mode {:?}; no isolation backend is available",
                self.mode
            )));
        }
        Ok(())
    }
}
