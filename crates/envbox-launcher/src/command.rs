//! Command resolution for LaunchTarget Command / Executable.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommandError {
    #[error("command not found: {0}")]
    CommandNotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: PathBuf,
    /// True when the target is a .cmd/.bat wrapper that must go through `%ComSpec% /d /s /c`.
    pub via_comspec: bool,
    /// For ComSpec: the raw command string placed after `/c`.
    pub comspec_payload: Option<String>,
}

const PATH_EXTS: [&str; 5] = ["", ".exe", ".com", ".cmd", ".bat"];

/// PATH resolution order: absolute/relative path, then PATH `.exe`/`.com`/`.cmd`/`.bat`.
/// `.cmd`/`.bat` are wrapped with `%ComSpec% /d /s /c`.
/// Only existing files are accepted (directories never resolve).
pub fn resolve_command(command: &str, path_env: Option<&str>) -> Result<ResolvedCommand, CommandError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(CommandError::CommandNotFound(command.to_string()));
    }

    let candidate = Path::new(trimmed);
    if candidate.is_absolute() || trimmed.contains('\\') || trimmed.contains('/') {
        return classify_path(candidate, trimmed);
    }

    let search = path_env.unwrap_or("");
    for dir in std::env::split_paths(search) {
        for ext in PATH_EXTS {
            let name = if ext.is_empty() || trimmed.to_ascii_lowercase().ends_with(ext) {
                trimmed.to_string()
            } else {
                format!("{trimmed}{ext}")
            };
            let full = dir.join(&name);
            if full.is_file() {
                return classify_path(&full, trimmed);
            }
        }
    }

    Err(CommandError::CommandNotFound(command.to_string()))
}

fn classify_path(path: &Path, original: &str) -> Result<ResolvedCommand, CommandError> {
    // Only real files; directories and missing paths fail closed.
    if !path.is_file() {
        return Err(CommandError::CommandNotFound(original.to_string()));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext == "cmd" || ext == "bat" {
        return Ok(ResolvedCommand {
            program: path.to_path_buf(),
            via_comspec: true,
            comspec_payload: Some(original.to_string()),
        });
    }
    Ok(ResolvedCommand {
        program: path.to_path_buf(),
        via_comspec: false,
        comspec_payload: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_command_reports_not_found() {
        let err = resolve_command("definitely-not-a-real-cmd-xyz", Some("")).unwrap_err();
        assert!(matches!(err, CommandError::CommandNotFound(_)));
    }

    #[test]
    fn empty_command_rejected() {
        assert!(resolve_command("  ", Some("")).is_err());
    }

    #[test]
    fn missing_explicit_cmd_path_rejected() {
        let err = resolve_command(r"Z:\nope\not-there.cmd", Some("")).unwrap_err();
        assert!(matches!(err, CommandError::CommandNotFound(_)));
    }
}
