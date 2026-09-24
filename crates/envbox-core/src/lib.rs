//! EnvBox domain model: Application, Environment Profile, RuntimeInstance.
//! Vocabulary follows `docs/CONTEXT.md`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::SystemTime;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid Environment Profile: {0}")]
    InvalidProfile(String),
    #[error("invalid Application: {0}")]
    InvalidApplication(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchTarget {
    Executable { path: PathBuf },
    Command { command: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    pub id: Uuid,
    pub name: String,
    pub launch: LaunchTarget,
    pub working_directory: Option<PathBuf>,
    pub arguments: Vec<String>,
    pub default_profile_id: Uuid,
    pub inherit_children: bool,
    /// Audit Mode default for this Application (ticket 20). CLI `--audit` forces on.
    #[serde(default)]
    pub audit: bool,
}

impl Application {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::InvalidApplication(
                "name must not be empty".into(),
            ));
        }
        match &self.launch {
            LaunchTarget::Executable { path } if path.as_os_str().is_empty() => {
                return Err(DomainError::InvalidApplication(
                    "executable path must not be empty".into(),
                ));
            }
            LaunchTarget::Command { command } if command.trim().is_empty() => {
                return Err(DomainError::InvalidApplication(
                    "command must not be empty".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleProfile {
    pub locale_name: String,
    pub ui_language: String,
    pub region: String,
}

impl LocaleProfile {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.locale_name.trim().is_empty() {
            return Err(DomainError::InvalidProfile(
                "locale_name must not be empty".into(),
            ));
        }
        if self.ui_language.trim().is_empty() {
            return Err(DomainError::InvalidProfile(
                "ui_language must not be empty".into(),
            ));
        }
        let region = self.region.trim();
        if region.len() != 2 || !region.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(DomainError::InvalidProfile(format!(
                "region must be a 2-letter ISO code, got {:?}",
                self.region
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimezoneProfile {
    pub windows_id: String,
    pub iana_id: String,
}

impl TimezoneProfile {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.windows_id.trim().is_empty() {
            return Err(DomainError::InvalidProfile(
                "timezone.windows_id must not be empty".into(),
            ));
        }
        if self.iana_id.trim().is_empty() {
            return Err(DomainError::InvalidProfile(
                "timezone.iana_id must not be empty".into(),
            ));
        }
        if self.windows_id.contains('\0') || self.iana_id.contains('\0') {
            return Err(DomainError::InvalidProfile(
                "timezone ids must not contain NUL".into(),
            ));
        }
        if !looks_like_iana_id(&self.iana_id) {
            return Err(DomainError::InvalidProfile(format!(
                "timezone.iana_id must be an IANA name (e.g. America/Los_Angeles), got {:?}",
                self.iana_id
            )));
        }
        Ok(())
    }
}

/// IANA tz name: `/`-separated segments of [A-Za-z0-9_+-]
/// (UTC/GMT, single-token zones like EST/CET/HST, Area/Location, Etc/GMT+1).
/// Rejects Windows IDs like "Pacific Standard Time".
fn looks_like_iana_id(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() || id.contains('\0') || id.contains('\\') || id.contains(' ') {
        return false;
    }
    id.split('/').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '+')
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    Host,
    VirtualView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsProfile {
    pub mode: DnsMode,
    pub servers: Vec<IpAddr>,
}

impl DnsProfile {
    pub fn validate(&self) -> Result<(), DomainError> {
        match self.mode {
            DnsMode::Host => {}
            DnsMode::VirtualView => {
                if self.servers.is_empty() {
                    return Err(DomainError::InvalidProfile(
                        "DNS VirtualView requires at least one server".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RegistryProfile {
    pub whitelist_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProfile {
    pub id: Uuid,
    pub name: String,
    pub locale: LocaleProfile,
    pub timezone: TimezoneProfile,
    pub dns: DnsProfile,
    pub environment: HashMap<String, String>,
    pub registry: RegistryProfile,
}

impl EnvironmentProfile {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::InvalidProfile("name must not be empty".into()));
        }
        self.locale.validate()?;
        self.timezone.validate()?;
        self.dns.validate()?;
        for (key, _) in &self.environment {
            if !is_valid_env_name(key) {
                return Err(DomainError::InvalidProfile(format!(
                    "invalid environment variable name {key:?}"
                )));
            }
        }
        Ok(())
    }
}

fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('=') && !name.contains('\0')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceStatus {
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInstance {
    pub id: Uuid,
    pub application_id: Uuid,
    pub profile_id: Uuid,
    pub root_pid: u32,
    pub process_ids: HashSet<u32>,
    pub started_at: SystemTime,
    pub status: InstanceStatus,
}

/// Audit Mode event schema v1 (ticket 20). One JSON object per line.
/// Never carries file contents, tokens, or full environment blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub v: u32,
    pub ts_utc: String,
    pub pid: u32,
    pub ppid: u32,
    pub tid: u32,
    pub api: String,
    pub virtualized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl AuditEvent {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn to_json_line(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    pub fn parse_json_line(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_profile() -> EnvironmentProfile {
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
                servers: vec!["1.1.1.1".parse().unwrap()],
            },
            environment: HashMap::from([("LANG".into(), "en_US.UTF-8".into())]),
            registry: RegistryProfile::default(),
        }
    }

    #[test]
    fn valid_profile_accepted() {
        assert!(valid_profile().validate().is_ok());
    }

    #[test]
    fn region_must_be_iso_alpha2() {
        let mut p = valid_profile();
        p.locale.region = "USA".into();
        assert!(matches!(p.validate(), Err(DomainError::InvalidProfile(_))));
    }

    #[test]
    fn virtual_view_requires_servers() {
        let mut p = valid_profile();
        p.dns.servers.clear();
        assert!(matches!(p.validate(), Err(DomainError::InvalidProfile(_))));
    }

    #[test]
    fn host_dns_may_be_empty() {
        let mut p = valid_profile();
        p.dns.mode = DnsMode::Host;
        p.dns.servers.clear();
        assert!(p.validate().is_ok());
    }

    #[test]
    fn env_name_with_equals_rejected() {
        let mut p = valid_profile();
        p.environment.insert("BAD=NAME".into(), "x".into());
        assert!(matches!(p.validate(), Err(DomainError::InvalidProfile(_))));
    }

    #[test]
    fn empty_locale_name_rejected() {
        let mut p = valid_profile();
        p.locale.locale_name = "  ".into();
        assert!(matches!(p.validate(), Err(DomainError::InvalidProfile(_))));
    }

    #[test]
    fn windows_id_must_not_masquerade_as_iana() {
        let mut p = valid_profile();
        p.timezone.iana_id = "Pacific Standard Time".into();
        assert!(matches!(p.validate(), Err(DomainError::InvalidProfile(_))));
    }

    #[test]
    fn utc_is_valid_iana() {
        let mut p = valid_profile();
        p.timezone.iana_id = "UTC".into();
        assert!(p.validate().is_ok());
    }

    #[test]
    fn single_token_iana_accepted() {
        for name in ["EST", "MST", "CET", "HST", "GMT", "Etc/GMT+1"] {
            let mut p = valid_profile();
            p.timezone.iana_id = name.into();
            assert!(p.validate().is_ok(), "must accept IANA {name:?}");
        }
    }

    #[test]
    fn empty_command_rejected() {
        let app = Application {
            id: Uuid::nil(),
            name: "x".into(),
            launch: LaunchTarget::Command {
                command: "  ".into(),
            },
            working_directory: None,
            arguments: vec![],
            default_profile_id: Uuid::nil(),
            inherit_children: true,
            audit: false,
        };
        assert!(matches!(
            app.validate(),
            Err(DomainError::InvalidApplication(_))
        ));
    }

    #[test]
    fn command_launch_target_round_trips() {
        let target = LaunchTarget::Command {
            command: "claude".into(),
        };
        let text = toml::to_string(&Wrap(target.clone())).unwrap();
        assert!(text.contains("type = \"command\""));
        let back: Wrap = toml::from_str(&text).unwrap();
        assert_eq!(back.0, target);
    }

    #[test]
    fn audit_event_round_trips_json_line() {
        let ev = AuditEvent {
            v: AuditEvent::SCHEMA_VERSION,
            ts_utc: "2026-09-24T12:00:00.000Z".into(),
            pid: 10,
            ppid: 2,
            tid: 3,
            api: "GetDynamicTimeZoneInformation".into(),
            virtualized: true,
            summary: Some("Pacific Standard Time".into()),
        };
        let line = ev.to_json_line().unwrap();
        assert!(!line.contains('\n'));
        let back = AuditEvent::parse_json_line(&line).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn audit_event_summary_optional() {
        let ev = AuditEvent {
            v: 1,
            ts_utc: "2026-09-24T12:00:00.000Z".into(),
            pid: 1,
            ppid: 0,
            tid: 1,
            api: "EnvBoxAuditInit".into(),
            virtualized: false,
            summary: None,
        };
        let line = ev.to_json_line().unwrap();
        assert!(!line.contains("summary"));
        assert_eq!(AuditEvent::parse_json_line(&line).unwrap(), ev);
    }

    #[test]
    fn audit_event_rejects_garbage() {
        assert!(AuditEvent::parse_json_line("not-json").is_err());
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Wrap(LaunchTarget);
}
