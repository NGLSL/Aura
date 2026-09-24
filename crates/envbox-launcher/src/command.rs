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

/// PATH resolution order: absolute/relative path, then PATH `.exe`/`.com`/`.cmd`/`.bat`.
/// `.cmd`/`.bat` are wrapped with `%ComSpec% /d /s /c`.
/// Extension-less candidates must be PE images (npm shims are not; prefer `.cmd`).
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
    // Prefer explicit PE / script extensions over bare npm shims (no extension).
    const PATH_EXTS: [&str; 5] = [".exe", ".com", ".cmd", ".bat", ""];
    for dir in std::env::split_paths(search) {
        for ext in PATH_EXTS {
            let name = if ext.is_empty() || trimmed.to_ascii_lowercase().ends_with(ext) {
                trimmed.to_string()
            } else {
                format!("{trimmed}{ext}")
            };
            let full = dir.join(&name);
            if full.is_file() {
                if ext.is_empty() && !is_pe_image(&full) {
                    continue;
                }
                return classify_path(&full, trimmed);
            }
        }
    }

    Err(CommandError::CommandNotFound(command.to_string()))
}

fn is_pe_image(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut magic = [0u8; 2];
    matches!(f.read_exact(&mut magic), Ok(())) && magic == *b"MZ"
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

    #[test]
    fn prefers_cmd_wrapper_over_extensionless_npm_shim() {
        let dir = std::env::temp_dir().join(format!("envbox-resolve-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // Non-PE npm-style shim (no extension) must not win over .cmd.
        std::fs::write(dir.join("agent-shim"), b"#!/usr/bin/env node\n").unwrap();
        std::fs::write(dir.join("agent-shim.cmd"), b"@echo off\r\n").unwrap();
        let path = dir.display().to_string();
        let resolved = resolve_command("agent-shim", Some(&path)).unwrap();
        assert!(resolved.via_comspec, "must wrap .cmd via ComSpec");
        assert!(
            resolved.program.extension().map(|e| e == "cmd").unwrap_or(false),
            "got {:?}",
            resolved.program
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
