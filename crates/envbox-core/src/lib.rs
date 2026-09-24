//! EnvBox domain model: Application, Environment Profile, RuntimeInstance.
//! Vocabulary follows `docs/CONTEXT.md`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleProfile {
    pub locale_name: String,
    pub ui_language: String,
    pub region: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimezoneProfile {
    pub windows_id: String,
    pub iana_id: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_target_command_keeps_raw_command() {
        let target = LaunchTarget::Command {
            command: "claude".into(),
        };
        assert!(matches!(target, LaunchTarget::Command { command } if command == "claude"));
    }

    #[test]
    fn dns_mode_distinct() {
        assert_ne!(DnsMode::Host, DnsMode::VirtualView);
    }
}
