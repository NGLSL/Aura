//! EnvBox domain model: Application, Environment Profile, RuntimeInstance.
//! Vocabulary follows `docs/CONTEXT.md`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;
use thiserror::Error;
use uuid::Uuid;

pub mod browser_policy;
pub mod container;
pub use container::{Container, ContainerMode};
pub mod dns;
pub mod run_snapshot;
pub mod session;
pub mod storage_policy;
pub use dns::{DnsProfile, DnsTlsRevocation, DnsUpstream};
pub use run_snapshot::RunSnapshot;

pub use browser_policy::{
    browser_env_entries, ensure_chromium_webrtc_argv, ensure_chromium_webrtc_switch,
    ensure_webview2_arguments, plan_child_policy, BrowserChildPolicy, BrowserEngine,
    BrowserGuarantee, BrowserPrivacyProfile, CommandLinePolicy, NetworkGuardCapability,
    PolicyApply, WebRtcPolicy, AUDIT_API_NETWORK_UDP_DENY,
};
pub use session::{
    capabilities_for_target, evaluate_injection_support, is_packaged_target,
    isolation_for_strategy, select_attach_strategy, ActivatedTarget, ActivationType,
    AttachStrategy, EnvironmentSession, InjectionCapability, IntegrityLevel, IsolationGuarantee,
    MitigationPolicy, PackageIdentity, SessionState, TargetCapabilities,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid Container: {0}")]
    InvalidContainer(String),
    #[error("invalid Environment Profile: {0}")]
    InvalidProfile(String),
    #[error("invalid Application: {0}")]
    InvalidApplication(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchTarget {
    Executable {
        path: PathBuf,
    },
    Command {
        command: String,
    },
    /// Packaged / WindowsApps target activated by AUMID (never raw WindowsApps exe).
    Packaged {
        aumid: String,
        package_full_name: String,
        package_family_name: String,
    },
}

/// Console host used when an Application launches a command target.
///
/// This is an Application preference only.  The launcher is responsible for
/// applying the selected host; keeping it here lets the preference survive a
/// GUI restart and keeps older application documents compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleHost {
    /// Launch the target directly in the current process model.
    Direct,
    /// Run the command through cmd.exe.
    Cmd,
    /// Run the command through PowerShell.
    PowerShell,
    /// Open the command in Windows Terminal.
    WindowsTerminal,
}

impl Default for ConsoleHost {
    fn default() -> Self {
        Self::Direct
    }
}

impl ConsoleHost {
    pub const ALL: [Self; 4] = [
        Self::Direct,
        Self::Cmd,
        Self::PowerShell,
        Self::WindowsTerminal,
    ];
}

impl std::fmt::Display for ConsoleHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Direct => "Direct",
            Self::Cmd => "cmd.exe",
            Self::PowerShell => "PowerShell",
            Self::WindowsTerminal => "Windows Terminal",
        };
        f.write_str(label)
    }
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
    /// Console host preference for command targets. Older documents default
    /// to direct launch for backwards compatibility.
    #[serde(default)]
    pub console_host: ConsoleHost,
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
            LaunchTarget::Packaged { aumid, .. } if aumid.trim().is_empty() => {
                return Err(DomainError::InvalidApplication(
                    "packaged aumid must not be empty".into(),
                ));
            }
            _ => {}
        }
        if !matches!(self.console_host, ConsoleHost::Direct)
            && !matches!(self.launch, LaunchTarget::Command { .. })
        {
            return Err(DomainError::InvalidApplication(
                "non-direct console hosts are available only for command targets".into(),
            ));
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
    /// Browser / Network Guard (WebRTC Privacy). Default = Host (compat).
    #[serde(default)]
    pub browser: crate::browser_policy::BrowserPrivacyProfile,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<Uuid>,
    pub root_pid: u32,
    pub process_ids: HashSet<u32>,
    pub started_at: SystemTime,
    pub status: InstanceStatus,
    /// Packaged sessions (ticket 27 / packaged-v1): optional identity + strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_family_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aumid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation_guarantee: Option<IsolationGuarantee>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_strategy: Option<AttachStrategy>,
}

impl RuntimeInstance {
    /// Map an EnvironmentSession control-plane aggregate to the persisted
    /// run record. Session fields never drop isolation/attach/package.
    pub fn from_session(session: &session::EnvironmentSession, started_at: SystemTime) -> Self {
        Self {
            id: session.id,
            application_id: session.application_id,
            profile_id: session.profile_id,
            container_id: None,
            snapshot_id: None,
            root_pid: session.root_processes.iter().next().copied().unwrap_or(0),
            process_ids: session.processes.clone(),
            started_at,
            status: InstanceStatus::from_session_state(session.state),
            package_family_name: session
                .package_identity
                .as_ref()
                .map(|p| p.package_family_name.clone()),
            aumid: session.package_identity.as_ref().map(|p| p.aumid.clone()),
            isolation_guarantee: Some(session.isolation),
            attach_strategy: Some(session.attach_strategy),
        }
    }
}

impl InstanceStatus {
    pub fn from_session_state(state: session::SessionState) -> Self {
        use session::SessionState as S;
        match state {
            S::Created | S::Activated | S::Attached => InstanceStatus::Starting,
            S::Running => InstanceStatus::Running,
            S::Stopping => InstanceStatus::Stopping,
            S::Exited => InstanceStatus::Exited,
            S::Failed => InstanceStatus::Failed,
        }
    }
}

/// Audit Mode event schema v1 (ticket 20). One JSON object per line.
/// Never carries file contents, tokens, or full environment blocks.
/// `n` collapses a short-window burst of identical calls (IME typing under
/// Audit Mode). Absent or 1 means a single call.
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
    /// Process image name (e.g. `chrome.exe`). Present on new events; optional on old logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Collapsed call count for this line (default 1).
    #[serde(
        default = "audit_event_default_n",
        skip_serializing_if = "audit_event_n_is_one"
    )]
    pub n: u32,
}

fn audit_event_default_n() -> u32 {
    1
}

fn audit_event_n_is_one(n: &u32) -> bool {
    *n <= 1
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
                ..Default::default()
            },
            environment: HashMap::from([("LANG".into(), "en_US.UTF-8".into())]),
            registry: RegistryProfile::default(),
            browser: BrowserPrivacyProfile::default(),
        }
    }

    #[test]
    fn valid_profile_accepted() {
        assert!(valid_profile().validate().is_ok());
    }

    #[test]
    fn snapshot_digest_is_map_order_stable_and_binds_instance_identity() {
        let mut profile = valid_profile();
        profile.environment = HashMap::from([("A".into(), "1".into()), ("B".into(), "2".into())]);
        let container = Container::new("A", profile.id);
        let a = RunSnapshot::new(&container, &profile, Uuid::new_v4()).unwrap();
        profile.environment = HashMap::from([("B".into(), "2".into()), ("A".into(), "1".into())]);
        let b = RunSnapshot::new(&container, &profile, Uuid::new_v4()).unwrap();
        assert_eq!(a.configuration_id, b.configuration_id);
        assert_ne!(a.content_digest, b.content_digest);
        let mut changed = a.clone();
        changed.instance_id = b.instance_id;
        changed.snapshot_id = b.snapshot_id;
        assert!(changed.validate().is_err());
        let encoded = serde_json::to_string(&a).unwrap();
        let decoded: RunSnapshot = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded, a);
        let mut missing = serde_json::to_value(&a).unwrap();
        missing["effective_profile"]
            .as_object_mut()
            .unwrap()
            .remove("browser");
        assert!(serde_json::from_value::<RunSnapshot>(missing).is_err());
        let mut unknown = serde_json::to_value(&a).unwrap();
        unknown["effective_profile"]["dns"]
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<RunSnapshot>(unknown).is_err());
    }

    #[test]
    fn snapshot_binds_explicit_doh_revocation_policy_and_rejects_missing_or_unknown() {
        let mut profile = valid_profile();
        profile.dns = DnsProfile::typed(
            DnsMode::VirtualView,
            true,
            vec![DnsUpstream::Doh {
                url: "https://1.1.1.1/dns-query".into(),
                bootstrap_ips: vec![],
                tls_revocation: DnsTlsRevocation::StrictOffline,
            }],
        );
        let container = Container::new("DoH", profile.id);
        let snapshot = RunSnapshot::new(&container, &profile, Uuid::new_v4()).unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            encoded["effective_profile"]["dns"]["upstreams"][0]["tls_revocation"],
            "strict_offline"
        );
        assert_eq!(
            serde_json::from_value::<RunSnapshot>(encoded.clone()).unwrap(),
            snapshot
        );
        let mut changed = snapshot.clone();
        if let DnsUpstream::Doh { tls_revocation, .. } =
            &mut changed.effective_profile.dns.upstreams[0]
        {
            *tls_revocation = DnsTlsRevocation::Standard;
        }
        assert!(changed.validate().is_err());
        let mut missing = encoded.clone();
        missing["effective_profile"]["dns"]["upstreams"][0]
            .as_object_mut()
            .unwrap()
            .remove("tls_revocation");
        assert!(serde_json::from_value::<RunSnapshot>(missing).is_err());
        let mut unknown = encoded;
        unknown["effective_profile"]["dns"]["upstreams"][0]["tls_revocation"] =
            serde_json::json!("disabled");
        assert!(serde_json::from_value::<RunSnapshot>(unknown).is_err());
    }

    #[test]
    fn runtime_instance_legacy_record_defaults_container_identity() {
        let session = EnvironmentSession::new(
            Uuid::new_v4(),
            LaunchTarget::Command {
                command: "cmd.exe".into(),
            },
            Uuid::new_v4(),
            IsolationGuarantee::FullPreExecution,
            AttachStrategy::PreExecution,
        );
        let instance = RuntimeInstance::from_session(&session, SystemTime::now());
        let encoded = serde_json::to_string(&instance).unwrap();
        assert!(!encoded.contains("container_id") && !encoded.contains("snapshot_id"));
        let decoded: RuntimeInstance = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.container_id, None);
        assert_eq!(decoded.snapshot_id, None);
    }

    #[test]
    fn browser_policy_round_trips() {
        let mut p = valid_profile();
        p.browser.webrtc = WebRtcPolicy::Strict;
        let json = serde_json::to_string(&p).unwrap();
        let back: EnvironmentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.browser.webrtc, WebRtcPolicy::Strict);
    }

    #[test]
    fn browser_defaults_when_missing() {
        let mut p = valid_profile();
        p.browser = BrowserPrivacyProfile::default();
        let mut json = serde_json::to_value(&p).unwrap();
        json.as_object_mut().unwrap().remove("browser");
        let back: EnvironmentProfile = serde_json::from_value(json).unwrap();
        assert_eq!(back.browser.webrtc, WebRtcPolicy::Host);
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
            console_host: ConsoleHost::Direct,
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
    fn application_console_host_defaults_for_legacy_documents() {
        let value = serde_json::json!({
            "id": Uuid::nil(),
            "name": "Claude Code",
            "launch": {"type": "command", "command": "claude"},
            "working_directory": null,
            "arguments": [],
            "default_profile_id": Uuid::nil(),
            "inherit_children": true,
            "audit": false
        });
        let app: Application = serde_json::from_value(value).unwrap();
        assert_eq!(app.console_host, ConsoleHost::Direct);
    }

    #[test]
    fn non_direct_console_hosts_require_command_target() {
        for console_host in [
            ConsoleHost::Cmd,
            ConsoleHost::PowerShell,
            ConsoleHost::WindowsTerminal,
        ] {
            let app = Application {
                id: Uuid::nil(),
                name: "GUI".into(),
                launch: LaunchTarget::Executable {
                    path: r"C:\\Windows\\System32\\notepad.exe".into(),
                },
                working_directory: None,
                arguments: vec![],
                default_profile_id: Uuid::nil(),
                inherit_children: true,
                console_host,
                audit: false,
            };
            assert!(matches!(
                app.validate(),
                Err(DomainError::InvalidApplication(message))
                    if message.contains("only for command targets")
            ));
        }
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
            image: Some("probe.exe".into()),
            n: 1,
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
            image: None,
            n: 1,
        };
        let line = ev.to_json_line().unwrap();
        assert!(!line.contains("summary"));
        assert_eq!(AuditEvent::parse_json_line(&line).unwrap(), ev);
    }

    #[test]
    fn audit_event_parses_legacy_line_without_image() {
        let legacy = r#"{"v":1,"ts_utc":"2026-09-24T12:00:00.000Z","pid":1,"ppid":0,"tid":1,"api":"GetTimeZoneInformation","virtualized":true}"#;
        let ev = AuditEvent::parse_json_line(legacy).unwrap();
        assert!(ev.image.is_none());
        assert_eq!(ev.n, 1);
        // Collapsed burst keeps n and still parses on legacy readers (n default).
        let burst = r#"{"v":1,"ts_utc":"2026-09-24T12:00:00.000Z","pid":1,"ppid":0,"tid":1,"api":"GetUserDefaultLCID","virtualized":true,"n":42}"#;
        let b = AuditEvent::parse_json_line(burst).unwrap();
        assert_eq!(b.n, 42);
        assert!(!b.to_json_line().unwrap().contains("\"n\":1"));
    }

    #[test]
    fn audit_event_rejects_garbage() {
        assert!(AuditEvent::parse_json_line("not-json").is_err());
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Wrap(LaunchTarget);
}
