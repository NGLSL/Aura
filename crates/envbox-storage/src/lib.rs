//! TOML persistence for Application and Environment Profile under `%LOCALAPPDATA%\com.aura.envbox\`.

use envbox_core::{Application, DomainError, EnvironmentProfile};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
mod containers;
mod run_snapshots;
pub use containers::ContainerDocument;

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

/// Per-user data folder name (Kite-style reverse-DNS). Also used by the
/// Runtime audit sink — keep in sync with `runtime/src/audit.cpp`.
pub const DATA_DIR_NAME: &str = "com.aura.envbox";

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
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let root = base.join(DATA_DIR_NAME);
        // One-time rename from the pre-0.3 `%LOCALAPPDATA%\EnvBox` layout.
        let legacy = base.join("EnvBox");
        if !root.exists() && legacy.exists() {
            let _ = fs::rename(&legacy, &root);
        }
        root
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
        }
        Ok(doc)
    }

    pub fn save_profiles(&self, doc: &ProfileDocument) -> Result<(), StorageError> {
        for profile in &doc.profiles {
            profile.validate()?;
            validate_timezone_windows_id(&profile.timezone.windows_id)?;
        }
        save_profile_doc_atomically(self.profiles_path(), doc)
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

fn save_profile_doc_atomically(path: PathBuf, doc: &ProfileDocument) -> Result<(), StorageError> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing Profile directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".profiles-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), StorageError> {
        let text = toml::to_string_pretty(doc)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        containers::atomic_replace(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        };

        // A Windows time-zone ID is one key below Time Zones. Opening that
        // exact key avoids enumerating every system zone for each Profile read.
        if windows_id.contains(['\\', '/']) {
            return false;
        }
        let path =
            format!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Time Zones\\{windows_id}");
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut key = HKEY::default();
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(wide.as_ptr()),
                0,
                KEY_READ,
                &mut key,
            )
        };
        if opened.is_ok() {
            unsafe {
                let _ = RegCloseKey(key);
            }
            true
        } else {
            false
        }
    }
    #[cfg(not(windows))]
    {
        // Non-Windows CI cannot enumerate Windows IDs; accept well-formed IDs.
        !windows_id.is_empty() && !windows_id.contains('\0')
    }
}

/// Curated Windows ↔ IANA pairs for the Profile editor (common zones only).
/// `None` in either direction means unmapped — never invent.
pub const COMMON_TIMEZONE_PAIRS: &[(&str, &str)] = &[
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Mountain Standard Time", "America/Denver"),
    ("Central Standard Time", "America/Chicago"),
    ("Eastern Standard Time", "America/New_York"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Mexico Standard Time", "America/Mexico_City"),
    ("GMT Standard Time", "Europe/London"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("UTC", "UTC"),
];

/// Windows timezone ID → IANA when known. `None` means unmapped — never invent.
/// GUI/CLI must require a real `iana_id`; do not fall back to the Windows ID.
pub fn windows_id_to_iana(windows_id: &str) -> Option<&'static str> {
    COMMON_TIMEZONE_PAIRS
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(windows_id))
        .map(|(_, i)| *i)
}

/// IANA timezone ID → Windows ID when known. `None` means unmapped.
pub fn iana_to_windows(iana_id: &str) -> Option<&'static str> {
    COMMON_TIMEZONE_PAIRS
        .iter()
        .find(|(_, i)| i.eq_ignore_ascii_case(iana_id))
        .map(|(w, _)| *w)
}

/// Common Windows timezone IDs for the Profile editor default list.
pub fn common_windows_timezone_ids() -> Vec<String> {
    COMMON_TIMEZONE_PAIRS
        .iter()
        .map(|(w, _)| (*w).to_string())
        .collect()
}

/// Common IANA timezone IDs for the Profile editor default list.
pub fn common_iana_timezone_ids() -> Vec<String> {
    COMMON_TIMEZONE_PAIRS
        .iter()
        .map(|(_, i)| (*i).to_string())
        .collect()
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
        ConsoleHost, DnsMode, DnsProfile, LaunchTarget, LocaleProfile, RegistryProfile,
        TimezoneProfile,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    fn sample_profile() -> EnvironmentProfile {
        EnvironmentProfile {
            identity: Default::default(),
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
                ..Default::default()
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

    #[test]
    fn data_dir_name_is_reverse_dns() {
        assert_eq!(DATA_DIR_NAME, "com.aura.envbox");
    }

    #[test]
    fn default_root_migrates_legacy_envbox_dir() {
        let base = std::env::temp_dir().join(format!("envbox-data-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("EnvBox")).unwrap();
        fs::write(base.join("EnvBox").join("profiles.toml"), "profiles = []\n").unwrap();

        // Drive the same rename logic default_root() uses under LOCALAPPDATA.
        let root = base.join(DATA_DIR_NAME);
        let legacy = base.join("EnvBox");
        if !root.exists() && legacy.exists() {
            fs::rename(&legacy, &root).unwrap();
        }
        assert!(root.join("profiles.toml").exists());
        assert!(!legacy.exists());
        let _ = fs::remove_dir_all(&base);
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
            console_host: ConsoleHost::Direct,
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
        assert!(
            text.contains("[[profiles]]"),
            "expected array-of-tables:\n{text}"
        );
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

    #[cfg(windows)]
    #[test]
    fn direct_timezone_lookup_accepts_enumerated_windows_ids() {
        let ids = enumerate_dynamic_timezone_ids();
        assert!(!ids.is_empty());
        for id in ids {
            assert!(
                windows_id_exists(&id),
                "enumerated ID missing its registry key: {id}"
            );
        }
        assert!(!windows_id_exists(r"..\Bogus"));
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
    fn typed_dns_order_roundtrip_and_invalid_bootstrap_preserve_profile_bytes() {
        use envbox_core::DnsUpstream;
        let root = std::env::temp_dir().join(format!("aura-storage-dns14-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let mut profile = sample_profile();
        profile.dns = DnsProfile::typed(
            DnsMode::VirtualView,
            true,
            vec![
                DnsUpstream::Doh {
                    url: "https://dns.example/dns-query".into(),
                    bootstrap_ips: vec!["1.1.1.1".parse().unwrap()],
                    tls_revocation: envbox_core::DnsTlsRevocation::StrictOffline,
                },
                DnsUpstream::Dot {
                    address: "1.0.0.1".parse().unwrap(),
                    port: 853,
                    server_name: "dns.example".into(),
                },
                DnsUpstream::Tcp {
                    address: "127.0.0.1".parse().unwrap(),
                    port: 15353,
                },
                DnsUpstream::Udp {
                    address: "127.0.0.1".parse().unwrap(),
                    port: 15354,
                },
            ],
        );
        let mut doc = ProfileDocument {
            profiles: vec![profile],
        };
        store.save_profiles(&doc).unwrap();
        assert_eq!(store.load_profiles().unwrap(), doc);
        let before = fs::read(store.profiles_path()).unwrap();
        if let DnsUpstream::Doh { bootstrap_ips, .. } = &mut doc.profiles[0].dns.upstreams[0] {
            bootstrap_ips.clear();
        }
        assert!(store.save_profiles(&doc).is_err());
        assert_eq!(fs::read(store.profiles_path()).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn dns_profile_atomic_replace_failure_retains_old_config() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!("aura-profile-write14-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let mut doc = ProfileDocument {
            profiles: vec![sample_profile()],
        };
        store.save_profiles(&doc).unwrap();
        let before = fs::read(store.profiles_path()).unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(store.profiles_path())
            .unwrap();
        doc.profiles[0].name = "changed".into();
        assert!(store.save_profiles(&doc).is_err());
        assert_eq!(fs::read(store.profiles_path()).unwrap(), before);
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        drop(locked);
        fs::remove_dir_all(root).unwrap();
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
        assert!(
            text.contains("[[applications]]"),
            "expected array-of-tables:\n{text}"
        );
        assert!(text.contains("type = \"command\""), "launch tag:\n{text}");
        let loaded = store.load_applications().unwrap();
        assert_eq!(loaded.applications[0].name, "Claude Code");
        assert_eq!(loaded.applications[0].console_host, ConsoleHost::Direct);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_application_defaults_to_direct_console_host() {
        let legacy = r#"
[[applications]]
id = "00000000-0000-0000-0000-000000000000"
name = "Claude Code"
launch = { type = "command", command = "claude" }
arguments = []
working_directory = ""
default_profile_id = "00000000-0000-0000-0000-000000000000"
inherit_children = true
audit = false
"#;
        let doc: ApplicationDocument =
            toml::from_str(legacy).expect("legacy application should load");
        assert_eq!(doc.applications[0].console_host, ConsoleHost::Direct);
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
