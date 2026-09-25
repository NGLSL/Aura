//! TOML persistence for Application and Environment Profile under `%LOCALAPPDATA%\EnvBox\`.

use envbox_core::{Application, DomainError, EnvironmentProfile};
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
    #[error(transparent)]
    Domain(#[from] DomainError),
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
        if let Some(root) = std::env::var_os("ENVBOX_CONFIG_ROOT") {
            return PathBuf::from(root);
        }
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

    /// Audit Mode sink directory (ticket 20).
    pub fn audit_dir(&self) -> PathBuf {
        self.root.join("audit")
    }

    /// One JSONL file per RuntimeInstance.
    pub fn audit_path(&self, instance_id: &uuid::Uuid) -> PathBuf {
        self.audit_dir().join(format!("{instance_id}.jsonl"))
    }

    pub fn ensure_audit_dir(&self) -> Result<(), StorageError> {
        fs::create_dir_all(self.audit_dir())?;
        Ok(())
    }

    pub fn load_profiles(&self) -> Result<ProfileDocument, StorageError> {
        let doc: ProfileDocument = load_doc(self.profiles_path())?;
        // Corrupt profiles must fail closed on read (L15).
        for profile in &doc.profiles {
            validate_profile(profile)?;
            validate_timezone_windows_id(&profile.timezone.windows_id)?;
        }
        Ok(doc)
    }

    pub fn save_profiles(&self, doc: &ProfileDocument) -> Result<(), StorageError> {
        for profile in &doc.profiles {
            profile.validate()?;
            validate_timezone_windows_id(&profile.timezone.windows_id)?;
        }
        save_doc(self.profiles_path(), doc)
    }

    pub fn load_applications(&self) -> Result<ApplicationDocument, StorageError> {
        load_doc(self.applications_path())
    }

    pub fn save_applications(&self, doc: &ApplicationDocument) -> Result<(), StorageError> {
        for app in &doc.applications {
            app.validate()?;
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

/// Reject timezone Windows IDs that cannot exist on this host.
/// Format checks live in core; this is the host-existence gate at save time.
fn validate_timezone_windows_id(windows_id: &str) -> Result<(), DomainError> {
    let id = windows_id.trim();
    if id.is_empty() {
        return Err(DomainError::InvalidProfile(
            "timezone.windows_id must not be empty".into(),
        ));
    }
    if !windows_id_exists(id) {
        return Err(DomainError::InvalidProfile(format!(
            "timezone Windows ID not found on this host: {id:?}"
        )));
    }
    Ok(())
}

fn windows_id_exists(windows_id: &str) -> bool {
    #[cfg(windows)]
    {
        enumerate_dynamic_timezone_ids()
            .iter()
            .any(|id| id.eq_ignore_ascii_case(windows_id))
    }
    #[cfg(not(windows))]
    {
        // Non-Windows CI cannot enumerate Windows IDs; accept well-formed IDs.
        !windows_id.is_empty() && !windows_id.contains('\0')
    }
}

/// Windows timezone ID → IANA when known. `None` means unmapped — never invent.
/// GUI/CLI must require a real `iana_id`; do not fall back to the Windows ID.
pub fn windows_id_to_iana(windows_id: &str) -> Option<&'static str> {
    let map = [
        ("Pacific Standard Time", "America/Los_Angeles"),
        ("Mountain Standard Time", "America/Denver"),
        ("Central Standard Time", "America/Chicago"),
        ("Eastern Standard Time", "America/New_York"),
        ("China Standard Time", "Asia/Shanghai"),
        ("Tokyo Standard Time", "Asia/Tokyo"),
        ("GMT Standard Time", "Europe/London"),
        ("W. Europe Standard Time", "Europe/Berlin"),
        ("India Standard Time", "Asia/Kolkata"),
        ("Singapore Standard Time", "Asia/Singapore"),
        ("AUS Eastern Standard Time", "Australia/Sydney"),
        ("Korea Standard Time", "Asia/Seoul"),
        ("Taipei Standard Time", "Asia/Taipei"),
        ("Arabian Standard Time", "Asia/Dubai"),
        ("Israel Standard Time", "Asia/Jerusalem"),
        ("Russian Standard Time", "Europe/Moscow"),
        ("SA Pacific Standard Time", "America/Bogota"),
        ("E. South America Standard Time", "America/Sao_Paulo"),
        ("UTC", "UTC"),
    ];
    map.iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(windows_id))
        .map(|(_, i)| *i)
}

/// Windows timezone IDs from host enumeration (Profile editor dropdown).
#[cfg(windows)]
pub use win_tz::enumerate_dynamic_timezone_ids;

#[cfg(not(windows))]
pub fn enumerate_dynamic_timezone_ids() -> Vec<String> {
    vec!["Pacific Standard Time".into()]
}

#[cfg(windows)]
mod win_tz {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::Time::{
        EnumDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION,
    };

    pub fn enumerate_dynamic_timezone_ids() -> Vec<String> {
        let mut ids = Vec::new();
        let mut index = 0u32;
        loop {
            let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
            let status = unsafe { EnumDynamicTimeZoneInformation(index, &mut info) };
            // ERROR_NO_MORE_ITEMS = 259
            if status == 259 || status != 0 {
                break;
            }
            let name: String = {
                let len = info
                    .TimeZoneKeyName
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(info.TimeZoneKeyName.len());
                std::ffi::OsString::from_wide(&info.TimeZoneKeyName[..len])
                    .to_string_lossy()
                    .into_owned()
            };
            if !name.is_empty() {
                ids.push(name);
            }
            index += 1;
            if index > 1024 {
                break;
            }
        }
        ids
    }
}

/// Validate an Environment Profile including host timezone existence.
pub fn validate_profile(profile: &EnvironmentProfile) -> Result<(), StorageError> {
    profile.validate()?;
    validate_timezone_windows_id(&profile.timezone.windows_id)?;
    Ok(())
}

/// Validate an Application before persisting.
pub fn validate_application(app: &Application) -> Result<(), StorageError> {
    app.validate()?;
    Ok(())
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
            browser: Default::default(),
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
            audit: false,
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
        let text = std::fs::read_to_string(store.profiles_path()).unwrap();
        assert!(text.contains("[[profiles]]"), "expected array-of-tables:\n{text}");
        let loaded = store.load_profiles().unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].name, "US Development");
        assert_eq!(loaded.profiles[0].locale.region, "US");
        assert_eq!(
            loaded.profiles[0].timezone.windows_id,
            "Pacific Standard Time"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn browser_webrtc_round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("envbox-test-webrtc-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&dir);
        store.ensure_dirs().unwrap();
        let mut p = sample_profile();
        p.browser.webrtc = envbox_core::WebRtcPolicy::Strict;
        store
            .save_profiles(&ProfileDocument {
                profiles: vec![p.clone()],
            })
            .unwrap();
        let text = std::fs::read_to_string(store.profiles_path()).unwrap();
        assert!(text.contains("webrtc"), "expected webrtc in TOML:\n{text}");
        let loaded = store.load_profiles().unwrap();
        assert_eq!(
            loaded.profiles[0].browser.webrtc,
            envbox_core::WebRtcPolicy::Strict
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_region_is_rejected_before_save() {
        let mut profile = sample_profile();
        profile.locale.region = "USA".into();
        assert!(matches!(
            validate_profile(&profile),
            Err(StorageError::Domain(_))
        ));
    }

    #[test]
    fn unknown_timezone_windows_id_is_rejected() {
        let mut profile = sample_profile();
        profile.timezone.windows_id = "Not A Real Zone".into();
        assert!(matches!(
            validate_profile(&profile),
            Err(StorageError::Domain(_))
        ));
    }

    #[test]
    fn invalid_dns_servers_rejected_by_type_and_mode() {
        let mut profile = sample_profile();
        profile.dns.servers.clear();
        assert!(matches!(
            validate_profile(&profile),
            Err(StorageError::Domain(_))
        ));
    }

    #[test]
    fn windows_id_as_iana_is_rejected() {
        let mut profile = sample_profile();
        profile.timezone.iana_id = profile.timezone.windows_id.clone();
        assert!(matches!(
            validate_profile(&profile),
            Err(StorageError::Domain(_))
        ));
    }

    #[test]
    fn windows_id_to_iana_is_option_without_fake_fallback() {
        assert_eq!(
            windows_id_to_iana("Pacific Standard Time"),
            Some("America/Los_Angeles")
        );
        assert_eq!(windows_id_to_iana("Not A Real Zone"), None);
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
        let text = std::fs::read_to_string(store.applications_path()).unwrap();
        assert!(text.contains("[[applications]]"), "expected array-of-tables:\n{text}");
        assert!(text.contains("type = \"command\""), "launch tag:\n{text}");
        let loaded = store.load_applications().unwrap();
        assert_eq!(loaded.applications[0].name, "Claude Code");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_path_is_per_instance_jsonl() {
        let dir = std::env::temp_dir().join(format!("envbox-test-audit-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&dir);
        store.ensure_audit_dir().unwrap();
        let id = Uuid::new_v4();
        let path = store.audit_path(&id);
        assert_eq!(path, dir.join("audit").join(format!("{id}.jsonl")));
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
            Err(StorageError::Domain(_))
        ));
    }

    #[test]
    fn invalid_profile_is_not_written() {
        let dir = std::env::temp_dir().join(format!("envbox-test-nosave-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&dir);
        store.ensure_dirs().unwrap();
        let mut bad = sample_profile();
        bad.locale.region = "X".into();
        assert!(store
            .save_profiles(&ProfileDocument {
                profiles: vec![bad]
            })
            .is_err());
        assert!(!store.profiles_path().exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
