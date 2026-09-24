//! LaunchRequest → LaunchedProcess. No Runtime injection in this ticket.

use crate::command::{resolve_command, CommandError, ResolvedCommand};
use crate::environment::build_environment_block;
use crate::job::{InstanceJob, JobError};
use envbox_core::{EnvironmentProfile, LaunchTarget};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error("working directory does not exist: {0}")]
    WorkingDirectoryMissing(PathBuf),
    #[error("profile invalid: {0}")]
    InvalidProfile(String),
    #[error("failed to create process: {0}")]
    CreateProcess(String),
    #[error("comspec not set")]
    ComSpecMissing,
}

pub struct LaunchRequest {
    pub launch: LaunchTarget,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub profile: EnvironmentProfile,
    pub instance_id: Uuid,
}

pub struct LaunchedProcess {
    pub pid: u32,
    pub instance_id: Uuid,
    pub profile_id: Uuid,
    pub job: InstanceJob,
    #[cfg(windows)]
    pub child: std::process::Child,
}

impl LaunchedProcess {
    pub fn wait(&mut self) -> Result<std::process::ExitStatus, std::io::Error> {
        #[cfg(windows)]
        {
            return self.child.wait();
        }
        #[cfg(not(windows))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "wait is windows-only",
            ))
        }
    }

    pub fn stop(&mut self) -> Result<(), LaunchError> {
        self.job.terminate()?;
        #[cfg(windows)]
        {
            let _ = self.child.kill();
        }
        Ok(())
    }
}

/// Quote one Windows argument for CreateProcess command line (CommandLineToArgvW rules).
pub fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = arg.contains(' ') || arg.contains('\t') || arg.contains('"');
    if !needs_quotes {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(backslashes * 2));
    out.push('"');
    out
}

fn host_environment() -> HashMap<String, String> {
    std::env::vars().collect()
}

pub fn launch(req: LaunchRequest) -> Result<LaunchedProcess, LaunchError> {
    req.profile
        .validate()
        .map_err(|e| LaunchError::InvalidProfile(e.to_string()))?;

    if let Some(dir) = &req.working_directory {
        if !dir.is_dir() {
            return Err(LaunchError::WorkingDirectoryMissing(dir.clone()));
        }
    }

    let env = build_environment_block(
        &host_environment(),
        Some(&req.profile),
        req.instance_id,
        req.profile.id,
    );

    // PATH search uses the merged environment (Profile PATH overrides Host).
    let path_env = env.get("PATH").cloned();

    let (resolved, user_args) = match &req.launch {
        LaunchTarget::Executable { path } => {
            let resolved = classify_exe(path)?;
            (resolved, req.arguments.clone())
        }
        LaunchTarget::Command { command } => {
            // Split first token as the program; remaining CLI args stay in `arguments`.
            let resolved = resolve_command(command, path_env.as_deref())?;
            (resolved, req.arguments.clone())
        }
    };

    let mut job = InstanceJob::create()?;
    let (program, args) = spawn_args(&resolved, &user_args, &req.profile)?;

    let mut cmd = std::process::Command::new(&program);
    cmd.args(&args);
    if let Some(dir) = &req.working_directory {
        cmd.current_dir(dir);
    }
    cmd.env_clear();
    for (k, v) in &env {
        cmd.env(k, v);
    }

    let mut child = cmd
        .spawn()
        .map_err(|err| LaunchError::CreateProcess(err.to_string()))?;

    let pid = child.id();
    if let Err(err) = job.assign_pid(pid) {
        let _ = child.kill();
        return Err(err.into());
    }

    Ok(LaunchedProcess {
        pid,
        instance_id: req.instance_id,
        profile_id: req.profile.id,
        job,
        child,
    })
}

fn classify_exe(path: &std::path::Path) -> Result<ResolvedCommand, LaunchError> {
    if !path.is_file() {
        return Err(CommandError::CommandNotFound(path.display().to_string()).into());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(ResolvedCommand {
        program: path.to_path_buf(),
        via_comspec: ext == "cmd" || ext == "bat",
        comspec_payload: (ext == "cmd" || ext == "bat")
            .then(|| path.display().to_string()),
    })
}

fn spawn_args(
    resolved: &ResolvedCommand,
    user_args: &[String],
    profile: &EnvironmentProfile,
) -> Result<(PathBuf, Vec<String>), LaunchError> {
    if resolved.via_comspec {
        let comspec = profile
            .environment
            .get("ComSpec")
            .or_else(|| profile.environment.get("COMSPEC"))
            .cloned()
            .or_else(|| std::env::var("ComSpec").ok())
            .or_else(|| std::env::var("COMSPEC").ok())
            .ok_or(LaunchError::ComSpecMissing)?;
        let payload = resolved
            .comspec_payload
            .clone()
            .unwrap_or_else(|| resolved.program.display().to_string());
        let mut line = quote_arg(&payload);
        // drop outer quotes only if payload had none and no spaces — quote_arg handles it
        if !payload.contains(' ') && !payload.contains('\t') && !payload.contains('"') {
            line = payload.clone();
        }
        for arg in user_args {
            line.push(' ');
            line.push_str(&quote_arg(arg));
        }
        Ok((
            PathBuf::from(comspec),
            vec!["/d".into(), "/s".into(), "/c".into(), line],
        ))
    } else {
        Ok((resolved.program.clone(), user_args.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_arg_plain() {
        assert_eq!(quote_arg("foo"), "foo");
    }

    #[test]
    fn quote_arg_spaces() {
        assert_eq!(quote_arg("a b"), "\"a b\"");
    }

    #[test]
    fn quote_arg_empty() {
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn quote_arg_embedded_quote() {
        let q = quote_arg("a\"b");
        assert!(q.starts_with('"') && q.ends_with('"'));
    }
}
