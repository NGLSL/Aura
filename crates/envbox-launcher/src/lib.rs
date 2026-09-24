//! Launch pipeline: command resolution, Environment Block merge, process lifecycle.
//! Injection lands in later tickets; this module owns the pure launch contracts first.

use envbox_core::EnvironmentProfile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("command not found: {0}")]
    CommandNotFound(String),
    #[error("working directory does not exist: {0}")]
    WorkingDirectoryMissing(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    /// True when the target is a .cmd/.bat wrapper that must go through `%ComSpec% /d /s /c`.
    pub via_comspec: bool,
}

/// PATH resolution order: absolute/relative EXE, then PATH `.exe`/`.com`/`.cmd`/`.bat`.
/// `.cmd`/`.bat` are wrapped later with `%ComSpec% /d /s /c "<command>"`.
pub fn resolve_command(command: &str, path_env: Option<&str>) -> Result<ResolvedCommand, LaunchError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(LaunchError::CommandNotFound(command.to_string()));
    }

    let candidate = Path::new(trimmed);
    if candidate.is_absolute() || trimmed.contains('\\') || trimmed.contains('/') {
        return classify_path(candidate, trimmed);
    }

    let search = path_env.unwrap_or("");
    for dir in std::env::split_paths(search) {
        for ext in ["", ".exe", ".com", ".cmd", ".bat"] {
            let mut name = trimmed.to_string();
            if ext.is_empty() {
                // try as-is first when ext already present in the search loop via "" pass
            } else if !trimmed.to_ascii_lowercase().ends_with(ext) {
                name = format!("{trimmed}{ext}");
            } else {
                name = trimmed.to_string();
            }
            let full = dir.join(&name);
            if full.is_file() {
                return classify_path(&full, trimmed);
            }
        }
    }

    Err(LaunchError::CommandNotFound(command.to_string()))
}

fn classify_path(path: &Path, command: &str) -> Result<ResolvedCommand, LaunchError> {
    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let via_comspec = ext == "cmd" || ext == "bat";
        return Ok(ResolvedCommand {
            program: path.to_path_buf(),
            arguments: Vec::new(),
            via_comspec,
        });
    }
    // Also accept bare names that still need PATH-style classification when the path exists after ext.
    let lower = command.to_ascii_lowercase();
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        return Ok(ResolvedCommand {
            program: path.to_path_buf(),
            arguments: Vec::new(),
            via_comspec: true,
        });
    }
    if path.exists() {
        return Ok(ResolvedCommand {
            program: path.to_path_buf(),
            arguments: Vec::new(),
            via_comspec: false,
        });
    }
    Err(LaunchError::CommandNotFound(command.to_string()))
}

/// Clone host environment, apply Profile overrides, then EnvBox internal IDs.
pub fn build_environment_block(
    host: &HashMap<String, String>,
    profile: Option<&EnvironmentProfile>,
    instance_id: Uuid,
    profile_id: Uuid,
) -> HashMap<String, String> {
    let mut env = host.clone();
    if let Some(profile) = profile {
        for (key, value) in &profile.environment {
            env.insert(key.clone(), value.clone());
        }
    }
    env.insert("ENVBOX_INSTANCE_ID".into(), instance_id.to_string());
    env.insert("ENVBOX_PROFILE_ID".into(), profile_id.to_string());
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::{
        DnsMode, DnsProfile, LocaleProfile, RegistryProfile, TimezoneProfile,
    };

    #[test]
    fn missing_command_reports_not_found() {
        let err = resolve_command("definitely-not-a-real-cmd-xyz", Some("")).unwrap_err();
        assert!(matches!(err, LaunchError::CommandNotFound(_)));
    }

    #[test]
    fn environment_block_applies_profile_and_internal_ids() {
        let profile = EnvironmentProfile {
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
                mode: DnsMode::Host,
                servers: vec![],
            },
            environment: HashMap::from([("LANG".into(), "en_US.UTF-8".into())]),
            registry: RegistryProfile::default(),
        };
        let host = HashMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("PATH".into(), r"C:\Windows".into()),
        ]);
        let merged = build_environment_block(&host, Some(&profile), Uuid::nil(), Uuid::nil());
        assert_eq!(merged.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
        assert_eq!(merged.get("PATH").map(String::as_str), Some(r"C:\Windows"));
        assert!(merged.contains_key("ENVBOX_INSTANCE_ID"));
        assert!(merged.contains_key("ENVBOX_PROFILE_ID"));
    }
}
