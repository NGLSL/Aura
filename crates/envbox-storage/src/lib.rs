//! TOML persistence for Application and Environment Profile under `%LOCALAPPDATA%\EnvBox\`.

use envbox_core::{Application, EnvironmentProfile};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("toml decode error: {0}")]
    TomlDecode(#[from] toml::de::Error),
    #[error("toml encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("profile validation failed: {0}")]
    InvalidProfile(String),
    #[error("application validation failed: {0}")]
    InvalidApplication(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct ProfileDocument {
    pub profiles: Vec<EnvironmentProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct ApplicationDocument {
    pub applications: Vec<Application>,
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn default_root() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("EnvBox")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn profiles_path(&self) -> PathBuf {
        self.root.join("profiles.toml")
    }

    pub fn applications_path(&self) -> PathBuf {
        self.root.join("applications.toml")
    }

    pub fn ensure_dirs(&self) -> Result<(), StorageError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.root.join("logs"))?;
        Ok(())
    }

    pub fn load_profiles(&self) -> Result<ProfileDocument, StorageError> {
        load_doc(self.profiles_path())
    }

    pub fn save_profiles(&self, doc: &ProfileDocument) -> Result<(), StorageError> {
        for profile in &doc.profiles {
            validate_profile(profile)?;
        }
        save_doc(self.profiles_path(), doc)
    }

    pub fn load_applications(&self) -> Result<ApplicationDocument, StorageError> {
        load_doc(self.applications_path())
    }

    pub fn save_applications(&self, doc: &ApplicationDocument) -> Result<(), StorageError> {
        for app in &doc.applications {
            validate_application(app)?;
        }
        save_doc(self.applications_path(), doc)
    }
}

fn load_doc<T: serde::de::DeserializeOwned + Default>(path: PathBuf) -> Result<T, StorageError> {
    if !path.exists() {
        return Ok(T::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

fn save_doc<T: serde::Serialize>(path: PathBuf, doc: &T) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(doc)?;
    fs::write(path, text)?;
    Ok(())
}

pub fn validate_profile(profile: &EnvironmentProfile) -> Result<(), StorageError> {
    if profile.name.trim().is_empty() {
        return Err(StorageError::InvalidProfile("name must not be empty".into()));
    }
    if profile.locale.locale_name.trim().is_empty() {
        return Err(StorageError::InvalidProfile("locale_name must not be empty".into()));
    }
    if profile.locale.ui_language.trim().is_empty() {
        return Err(StorageError::InvalidProfile("ui_language must not be empty".into()));
    }
    let region = profile.locale.region.trim();
    if region.len() != 2 || !region.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(StorageError::InvalidProfile(format!(
            "region must be a 2-letter ISO code, got {:?}",
            profile.locale.region
        )));
    }
    if profile.timezone.windows_id.trim().is_empty() {
        return Err(StorageError::InvalidProfile("timezone.windows_id must not be empty".into()));
    }
    if profile.timezone.iana_id.trim().is_empty() {
        return Err(StorageError::InvalidProfile("timezone.iana_id must not be empty".into()));
    }
    for (key, _) in &profile.environment {
        if !is_valid_env_name(key) {
            return Err(StorageError::InvalidProfile(format!(
                "invalid environment variable name {key:?}"
            )));
        }
    }
    Ok(())
}

pub fn validate_application(app: &Application) -> Result<(), StorageError> {
    if app.name.trim().is_empty() {
        return Err(StorageError::InvalidApplication("name must not be empty".into()));
    }
    match &app.launch {
        envbox_core::LaunchTarget::Executable { path } if path.as_os_str().is_empty() => {
            return Err(StorageError::InvalidApplication(
                "executable path must not be empty".into(),
            ));
        }
        envbox_core::LaunchTarget::Command { command } if command.trim().is_empty() => {
            return Err(StorageError::InvalidApplication(
                "command must not be empty".into(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn is_valid_env_name(name: &str) -> bool {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return false;
    }
    // Windows env names cannot contain `=`, and should be non-empty.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::{
        DnsMode, DnsProfile, LaunchTarget, LocaleProfile, RegistryProfile, TimezoneProfile,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    fn sample_profile() -> EnvironmentProfile {
        EnvironmentProfile {
            id: Uuid::nil(),
            name: "US Development".into(),
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
                mode: DnsMode::VirtualView,
                servers: vec!["1.1.1.1".parse().unwrap(), "1.0.0.1".parse().unwrap()],
            },
            environment: HashMap::from([
                ("LANG".into(), "en_US.UTF-8".into()),
                ("LC_ALL".into(), "en_US.UTF-8".into()),
                ("TZ".into(), "America/Los_Angeles".into()),
            ]),
            registry: RegistryProfile::default(),
        }
    }

    fn sample_app() -> Application {
        Application {
            id: Uuid::nil(),
            name: "Claude Code".into(),
            launch: LaunchTarget::Command {
                command: "claude".into(),
            },
            working_directory: Some(r"D:\Projects".into()),
            arguments: vec![],
            default_profile_id: Uuid::nil(),
            inherit_children: true,
        }
    }

    #[test]
    fn profile_round_trips_through_toml_document() {
        let dir = std::env::temp_dir().join(format!("envbox-test-profile-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&dir);
        store.ensure_dirs().unwrap();
        store
            .save_profiles(&ProfileDocument {
                profiles: vec![sample_profile()],
            })
            .unwrap();
        let loaded = store.load_profiles().unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].name, "US Development");
        assert_eq!(loaded.profiles[0].locale.region, "US");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_region_is_rejected_before_save() {
        let mut profile = sample_profile();
        profile.locale.region = "USA".into();
        assert!(matches!(
            validate_profile(&profile),
            Err(StorageError::InvalidProfile(_))
        ));
    }

    #[test]
    fn application_round_trips_through_toml_document() {
        let dir = std::env::temp_dir().join(format!("envbox-test-app-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&dir);
        store.ensure_dirs().unwrap();
        store
            .save_applications(&ApplicationDocument {
                applications: vec![sample_app()],
            })
            .unwrap();
        let loaded = store.load_applications().unwrap();
        assert_eq!(loaded.applications[0].name, "Claude Code");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_command_is_rejected() {
        let mut app = sample_app();
        app.launch = LaunchTarget::Command {
            command: "   ".into(),
        };
        assert!(matches!(
            validate_application(&app),
            Err(StorageError::InvalidApplication(_))
        ));
    }
}
