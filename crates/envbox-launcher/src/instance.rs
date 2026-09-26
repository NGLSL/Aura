//! RuntimeInstance lifecycle for GUI/CLI (ticket 10).
//! Job Object tracks the Process Tree Instance; not a security boundary.
//! Packaged / AUMID roots go through `start_session` (ActivationBackend).

use crate::job::{InstanceJob, JobError, JobStats};
use crate::launcher::LaunchError;
use crate::package_discovery::normalize_launch_target;
use crate::session::{
    start_session, start_session_in_new_console, SessionError, SessionHandle, SessionStartRequest,
};
use envbox_core::{Application, ConsoleHost, EnvironmentProfile, InstanceStatus, RuntimeInstance};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum InstanceError {
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("instance not found: {0}")]
    NotFound(Uuid),
    #[error("Windows Terminal launch failed: {0}")]
    WindowsTerminal(String),
    #[error("console host cannot safely represent the command arguments: {0}")]
    InvalidConsoleHost(String),
}

/// Run With target (ticket 11). Host is the real host configuration —
/// never a fabricated Profile (see docs/CONTEXT.md).
#[derive(Debug, Clone)]
pub enum RunTarget {
    Profile(EnvironmentProfile),
    /// Real host: no Profile overrides, no Runtime injection.
    Host,
}

impl RunTarget {
    /// Profile id on RuntimeInstance. Host is not a Profile (`Uuid::nil` = none).
    pub fn profile_id(&self) -> Uuid {
        match self {
            RunTarget::Profile(p) => p.id,
            RunTarget::Host => Uuid::nil(),
        }
    }
}

pub struct InstanceHandle {
    pub meta: RuntimeInstance,
    /// Session pipeline handle (activation/attach/IPC/job).
    pub session: Option<SessionHandle>,
    /// Job held by the GUI while Windows Terminal's hidden handoff starts.
    /// Once `terminal-run` opens the same named Job, this handle also tracks
    /// the real CLI root and its children. Direct sessions keep their Job in
    /// `session` for backwards compatibility with the normal pipeline.
    pub external_job: Option<InstanceJob>,
    /// WT handoff is resolved by the GUI refresh loop; launch itself must not
    /// block the Iced update thread while wt.exe starts its server/tab.
    handoff_deadline: Option<Instant>,
}

/// Process-scoped instance registry (GUI holds one of these).
#[derive(Default)]
pub struct InstanceManager {
    instances: HashMap<Uuid, InstanceHandle>,
}

impl InstanceManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run Application with a Profile or true Host (Run With; does not mutate
    /// `Application.default_profile_id`). Packaged/AUMID targets use
    /// `start_session` (never CreateProcess a WindowsApps exe).
    pub fn run(&mut self, app: &Application, target: RunTarget) -> Result<Uuid, InstanceError> {
        if app.console_host == ConsoleHost::WindowsTerminal {
            return self.run_windows_terminal(app, target);
        }
        let request = build_session_request(app, target)?;
        let session = if matches!(app.console_host, ConsoleHost::Cmd | ConsoleHost::PowerShell) {
            start_session_in_new_console(request)?
        } else {
            start_session(request)?
        };
        let id = session.instance.id;
        let meta = session.instance.clone();
        self.instances.insert(
            id,
            InstanceHandle {
                meta,
                session: Some(session),
                external_job: None,
                handoff_deadline: None,
            },
        );
        Ok(id)
    }

    /// Launch a command through the existing Windows Terminal client without
    /// putting the user command or its arguments on `wt.exe`'s command line.
    /// The tab runs `envbox.exe hidden terminal-run ...`; that short-lived
    /// command reads the saved Application/Profile and starts the actual CLI
    /// in the named Job created here.
    fn run_windows_terminal(
        &mut self,
        app: &Application,
        target: RunTarget,
    ) -> Result<Uuid, InstanceError> {
        if !matches!(app.launch, envbox_core::LaunchTarget::Command { .. }) {
            return Err(InstanceError::WindowsTerminal(
                "Windows Terminal is available only for command targets".into(),
            ));
        }

        let id = Uuid::new_v4();
        let job_name = format!("Local\\Aura-{id}");
        let job = InstanceJob::create_named(&job_name)?;
        let home = crate::launcher::effective_working_directory(
            &envbox_core::LaunchTarget::Command {
                command: "envbox-terminal-run".into(),
            },
            None,
        )
        .map_err(|err| InstanceError::WindowsTerminal(err.to_string()))?
        .ok_or_else(|| {
            InstanceError::WindowsTerminal("user profile directory unavailable".into())
        })?;
        let envbox = sibling_envbox_executable()?;
        let config_root = terminal_config_root();
        let config_fingerprint = terminal_config_fingerprint(&config_root);
        let args = windows_terminal_args(
            &envbox,
            &home,
            &config_root,
            config_fingerprint,
            id,
            app.id,
            target.profile_id(),
            &job_name,
        );

        let mut wt = spawn_windows_terminal(&args, &home).map_err(|err| {
            let _ = job.terminate();
            InstanceError::WindowsTerminal(err)
        })?;

        // The WT client normally hands the command to an existing server and
        // exits immediately. A non-zero client exit before the handoff is a
        // concrete launch failure; do not fall back to an unvirtualized CLI.
        if let Ok(Some(status)) = wt.try_wait() {
            if !status.success() {
                let _ = job.terminate();
                return Err(InstanceError::WindowsTerminal(format!(
                    "wt.exe exited with {status}"
                )));
            }
        }
        drop(wt);

        let profile_id = target.profile_id();
        let meta = RuntimeInstance {
            id,
            application_id: app.id,
            profile_id,
            root_pid: 0,
            process_ids: Default::default(),
            started_at: SystemTime::now(),
            status: InstanceStatus::Starting,
            package_family_name: None,
            aumid: None,
            isolation_guarantee: Some(envbox_core::IsolationGuarantee::FullPreExecution),
            attach_strategy: Some(envbox_core::AttachStrategy::PreExecution),
        };
        self.instances.insert(
            id,
            InstanceHandle {
                meta,
                session: None,
                external_job: Some(job),
                handoff_deadline: Some(Instant::now() + Duration::from_secs(10)),
            },
        );
        Ok(id)
    }

    /// Explicitly stop the Process Tree Instance through its Job Object.
    /// On failure status becomes Failed
    /// (never stuck at Stopping).
    pub fn stop(&mut self, id: Uuid) -> Result<(), InstanceError> {
        let Some(handle) = self.instances.get_mut(&id) else {
            return Err(InstanceError::NotFound(id));
        };
        if matches!(
            handle.meta.status,
            InstanceStatus::Exited | InstanceStatus::Failed
        ) {
            return Ok(());
        }
        handle.meta.status = InstanceStatus::Stopping;
        if handle.external_job.is_some() {
            // TerminateJobObject does not prevent a second process that still
            // holds the named handle from assigning a root a moment later.
            // Publish cancellation first; terminal-run checks this separate,
            // non-overwritable marker before and after activation so Stop
            // cannot be lost while it publishes the root-PID state marker.
            let _ = std::fs::write(
                crate::session::terminal_cancel_marker_path(handle.meta.id),
                "cancelled\n",
            );
        }
        let close_result = if let Some(job) = handle.external_job.as_mut() {
            Some(job.close())
        } else {
            handle
                .session
                .as_mut()
                .and_then(|session| session.job.as_mut().map(|j| j.close()))
        };
        match close_result {
            Some(Ok(())) | None => {
                handle.meta.status = InstanceStatus::Exited;
                Ok(())
            }
            Some(Err(err)) => {
                handle.meta.status = InstanceStatus::Failed;
                Err(err.into())
            }
        }
    }

    /// Refresh status from Job stats. Root exit with live children stays Running.
    pub fn refresh(&mut self, id: Uuid) -> Result<InstanceStatus, InstanceError> {
        let Some(handle) = self.instances.get_mut(&id) else {
            return Err(InstanceError::NotFound(id));
        };
        // Only terminal Failed/Exited freeze. Stopping must resolve via Job
        // (stop failure no longer leaves a permanent Stopping).
        if matches!(
            handle.meta.status,
            InstanceStatus::Exited | InstanceStatus::Failed
        ) {
            return Ok(handle.meta.status);
        }
        let job = handle.external_job.as_ref().or_else(|| {
            handle
                .session
                .as_ref()
                .and_then(|session| session.job.as_ref())
        });
        let Some(job) = job else {
            return Ok(handle.meta.status);
        };
        let stats = job.stats()?;
        let external_handoff = handle.external_job.is_some();
        let marker = external_handoff.then(|| terminal_root_marker(handle.meta.id));
        let marker_state = marker.as_deref().and_then(read_terminal_root_marker);
        if !stats.process_ids.is_empty() {
            // terminal-run publishes the exact root PID after the injected
            // session is created. Job PID list order is unspecified, so never
            // infer the root from the first entry.
            if let Some((root_pid, _)) = marker_state {
                handle.meta.root_pid = root_pid;
                handle.meta.status = InstanceStatus::Running;
            } else if handle.external_job.is_none() {
                // Direct sessions already have their root PID from the
                // activation result. WT sessions remain Starting until the
                // hidden handoff publishes that exact PID; Job PID order is
                // intentionally not used as a root guess.
                handle.meta.status = InstanceStatus::Running;
            }
            handle.meta.process_ids = stats.process_ids.iter().copied().collect();
        } else {
            handle.meta.process_ids.clear();
            if let Some((root_pid, exited)) = marker_state {
                handle.meta.root_pid = root_pid;
                if exited {
                    handle.meta.status = InstanceStatus::Exited;
                    if let Some(marker) = marker.as_deref() {
                        let _ = std::fs::remove_file(marker);
                    }
                }
            }
            if handle.meta.status == InstanceStatus::Starting
                && handle
                    .handoff_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                let _ = job.terminate();
                handle.meta.status = InstanceStatus::Failed;
                if let Some(marker) = marker.as_deref() {
                    let _ = std::fs::remove_file(marker);
                }
            }
        }
        let waiting_for_handoff =
            handle.meta.status == InstanceStatus::Starting && handle.handoff_deadline.is_some();
        if handle.meta.status != InstanceStatus::Failed && !waiting_for_handoff {
            handle.meta.status = status_from_stats(&stats, handle.meta.status);
        }
        Ok(handle.meta.status)
    }

    pub fn refresh_all(&mut self) {
        let ids: Vec<Uuid> = self.instances.keys().copied().collect();
        for id in ids {
            let _ = self.refresh(id);
        }
    }

    pub fn list(&self) -> Vec<RuntimeInstance> {
        let mut v: Vec<_> = self.instances.values().map(|h| h.meta.clone()).collect();
        v.sort_by_key(|m| m.started_at);
        v
    }

    pub fn get(&self, id: Uuid) -> Option<&RuntimeInstance> {
        self.instances.get(&id).map(|h| &h.meta)
    }

    pub fn child_count(&self, id: Uuid) -> Result<u32, InstanceError> {
        let Some(handle) = self.instances.get(&id) else {
            return Err(InstanceError::NotFound(id));
        };
        let job = handle.external_job.as_ref().or_else(|| {
            handle
                .session
                .as_ref()
                .and_then(|session| session.job.as_ref())
        });
        let Some(job) = job else {
            return Ok(0);
        };
        Ok(children_from_active_processes(
            job.stats()?.active_processes,
        ))
    }
}

fn terminal_root_marker(instance_id: Uuid) -> PathBuf {
    crate::session::terminal_root_marker_path(instance_id)
}

fn read_terminal_root_marker(path: &Path) -> Option<(u32, bool)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let root_pid = lines.next()?.trim().parse().ok()?;
    let exited = lines
        .next()
        .map(|state| state.trim() == "exited")
        .unwrap_or(false);
    Some((root_pid, exited))
}

fn sibling_envbox_executable() -> Result<PathBuf, InstanceError> {
    let current = std::env::current_exe().map_err(|err| {
        InstanceError::WindowsTerminal(format!("cannot resolve EnvBox executable directory: {err}"))
    })?;
    let path = current
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("envbox.exe");
    if !path.is_file() {
        return Err(InstanceError::WindowsTerminal(format!(
            "terminal-run executable not found beside GUI: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn windows_terminal_args(
    envbox: &Path,
    home: &Path,
    config_root: &Path,
    config_fingerprint: u64,
    instance_id: Uuid,
    application_id: Uuid,
    profile_id: Uuid,
    job_name: &str,
) -> Vec<String> {
    let profile = if profile_id.is_nil() {
        "host".to_string()
    } else {
        profile_id.to_string()
    };
    vec![
        "-w".into(),
        format!("Aura-{instance_id}"),
        "new-tab".into(),
        "--startingDirectory".into(),
        home.display().to_string(),
        envbox.display().to_string(),
        "hidden".into(),
        "terminal-run".into(),
        "--config-root".into(),
        config_root.display().to_string(),
        "--config-fingerprint".into(),
        config_fingerprint.to_string(),
        "--app-id".into(),
        application_id.to_string(),
        "--profile-id".into(),
        profile,
        "--instance-id".into(),
        instance_id.to_string(),
        "--job-name".into(),
        job_name.into(),
    ]
}

fn terminal_config_root() -> PathBuf {
    if let Some(root) = std::env::var_os("ENVBOX_CONFIG_ROOT") {
        return PathBuf::from(root);
    }
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("com.aura.envbox");
    }
    PathBuf::from(".").join("com.aura.envbox")
}

fn terminal_config_fingerprint(root: &Path) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for name in ["applications.toml", "profiles.toml"] {
        for byte in name.as_bytes().iter().copied().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        if let Ok(bytes) = std::fs::read(root.join(name)) {
            for byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

#[cfg(windows)]
fn spawn_windows_terminal(args: &[String], home: &Path) -> Result<std::process::Child, String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("wt.exe")
        .args(args)
        .current_dir(home)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|err| format!("cannot start wt.exe: {err}"))
}

#[cfg(not(windows))]
fn spawn_windows_terminal(_args: &[String], _home: &Path) -> Result<std::process::Child, String> {
    Err("Windows Terminal is available only on Windows".into())
}

/// Job `active_processes` includes Root Process; story 19 reports children only.
fn children_from_active_processes(active: u32) -> u32 {
    active.saturating_sub(1)
}

/// Build SessionStartRequest. Host → `profile: None` (no injection).
/// AUMID / shell:AppsFolder paths normalize to `LaunchTarget::Packaged`.
fn build_session_request(
    app: &Application,
    target: RunTarget,
) -> Result<SessionStartRequest, InstanceError> {
    let (profile, inherit_children) = match &target {
        RunTarget::Profile(p) => (Some(p.clone()), app.inherit_children),
        RunTarget::Host => (None, app.inherit_children),
    };
    let path_env = target_path_environment(&target);
    let (launch, arguments) = console_host_target_with_path(app, path_env.as_deref())?;
    Ok(SessionStartRequest {
        application_id: app.id,
        launch,
        arguments,
        working_directory: app.working_directory.clone(),
        profile,
        inherit_children,
        audit: app.audit,
    })
}

/// Materialize the selected console host into the Session request. This keeps
/// Cmd/PowerShell semantics in the same injected process tree as Direct, while
/// Windows Terminal uses the separate named-Job handoff above.
#[cfg(test)]
fn console_host_target(
    app: &Application,
) -> Result<(envbox_core::LaunchTarget, Vec<String>), InstanceError> {
    console_host_target_with_path(app, None)
}

fn console_host_target_with_path(
    app: &Application,
    path_env: Option<&str>,
) -> Result<(envbox_core::LaunchTarget, Vec<String>), InstanceError> {
    let launch = normalize_launch_target(&app.launch);
    let envbox_core::LaunchTarget::Command { command } = &launch else {
        return Ok((launch, app.arguments.clone()));
    };

    // Resolve the inner command with EnvBox's normal `.exe`/`.com`/`.cmd`/
    // `.bat` precedence before handing it to a shell host. PowerShell may
    // otherwise select a same-named `.ps1` shim while the normal launcher
    // deliberately prefers the `.cmd` npm wrapper. Tests can pass `None` to
    // exercise quoting without depending on a machine PATH.
    let shell_command = || {
        path_env
            .map(|path| crate::command::resolve_command(command, Some(path)))
            .transpose()
            .map_err(|err| {
                InstanceError::InvalidConsoleHost(format!(
                    "cannot resolve command {command:?} for shell host: {err}"
                ))
            })
            .map(|resolved| {
                resolved
                    .map(|resolved| resolved.program.to_string_lossy().into_owned())
                    .unwrap_or_else(|| command.clone())
            })
    };

    match app.console_host {
        ConsoleHost::Cmd => {
            // Percent expansion happens before cmd's quote handling. Refuse
            // such input instead of letting `%NAME%` become a different
            // argument or a command fragment. Delayed expansion is disabled
            // explicitly below, so literal `!` remains safe.
            if std::iter::once(command.as_str())
                .chain(app.arguments.iter().map(String::as_str))
                .any(|value| {
                    value
                        .chars()
                        .any(|ch| matches!(ch, '%' | '!' | '"' | '\r' | '\n' | '\0'))
                })
            {
                return Err(InstanceError::InvalidConsoleHost(
                    "cmd.exe mode cannot preserve %, !, quotes, or control characters in arguments"
                        .into(),
                ));
            }
            let shell_command = shell_command()?;
            if shell_command
                .chars()
                .any(|ch| matches!(ch, '%' | '!' | '"' | '\r' | '\n' | '\0'))
            {
                return Err(InstanceError::InvalidConsoleHost(
                    "cmd.exe mode cannot preserve the resolved command path".into(),
                ));
            }
            let mut payload = quote_cmd_token(&shell_command);
            for argument in &app.arguments {
                payload.push(' ');
                payload.push_str(&quote_cmd_token(argument));
            }
            // cmd /s /c removes the first and last quote. Keep an outer
            // pair so a quoted executable path remains intact after that.
            if payload.contains('"') {
                payload = format!("\"{payload}\"");
            }
            Ok((
                envbox_core::LaunchTarget::Command {
                    command: "cmd.exe".into(),
                },
                vec![
                    "/d".into(),
                    "/v:off".into(),
                    "/s".into(),
                    "/c".into(),
                    payload,
                ],
            ))
        }
        ConsoleHost::PowerShell => {
            let shell_command = shell_command()?;
            let batch = Path::new(&shell_command)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat")
                });
            if batch
                && std::iter::once(shell_command.as_str())
                    .chain(app.arguments.iter().map(String::as_str))
                    .any(|argument| {
                        argument.chars().any(|ch| {
                            matches!(
                                ch,
                                '%' | '!'
                                    | '"'
                                    | '&'
                                    | '|'
                                    | '<'
                                    | '>'
                                    | '^'
                                    | '('
                                    | ')'
                                    | '\r'
                                    | '\n'
                                    | '\0'
                            )
                        })
                    })
            {
                return Err(InstanceError::InvalidConsoleHost(
                    "PowerShell batch-wrapper mode cannot preserve cmd metacharacters in arguments"
                        .into(),
                ));
            }
            // Windows PowerShell 5.1 rebuilds a native command line when `&`
            // invokes an exe. Its legacy serializer merges an argument ending
            // in '\\' with the next argument. ProcessStartInfo.Arguments gives
            // the target the exact Windows CRT quoting we construct here.
            let (program, arguments) = if batch {
                let mut payload = quote_cmd_token(&shell_command);
                for argument in &app.arguments {
                    payload.push(' ');
                    payload.push_str(&quote_cmd_token(argument));
                }
                if payload.contains('"') {
                    payload = format!("\"{payload}\"");
                }
                ("cmd.exe".to_string(), format!("/d /v:off /s /c {payload}"))
            } else {
                (
                    shell_command,
                    app.arguments
                        .iter()
                        .map(|argument| crate::launcher::quote_arg(argument))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            };
            let script = format!(
                "$p = New-Object System.Diagnostics.ProcessStartInfo; $p.FileName = {}; $p.Arguments = {}; $p.UseShellExecute = $false; $c = [System.Diagnostics.Process]::Start($p); $c.WaitForExit(); exit $c.ExitCode",
                quote_powershell_literal(&program),
                quote_powershell_literal(&arguments),
            );
            Ok((
                envbox_core::LaunchTarget::Command {
                    command: "powershell.exe".into(),
                },
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-Command".into(),
                    script,
                ],
            ))
        }
        ConsoleHost::Direct | ConsoleHost::WindowsTerminal => Ok((launch, app.arguments.clone())),
    }
}

fn target_path_environment(target: &RunTarget) -> Option<String> {
    let host = std::env::var("PATH").ok();
    match target {
        RunTarget::Host => host,
        RunTarget::Profile(profile) => profile
            .environment
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value.clone())
            .or(host),
    }
}

fn quote_cmd_token(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".into();
    }
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '&' | '|' | '<' | '>' | '^' | '(' | ')'))
    {
        let trailing_slashes = value.chars().rev().take_while(|ch| *ch == '\\').count();
        format!("\"{}{}\"", value, "\\".repeat(trailing_slashes))
    } else {
        value.to_string()
    }
}

fn quote_powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Job has live processes -> Running; empty job after Running/Starting -> Exited
/// (fast-exit root is Exited, not Failed). Stop failure sets Failed directly.
fn status_from_stats(stats: &JobStats, prev: InstanceStatus) -> InstanceStatus {
    match prev {
        InstanceStatus::Starting | InstanceStatus::Running | InstanceStatus::Stopping => {
            if stats.active_processes > 0 {
                match prev {
                    InstanceStatus::Stopping => InstanceStatus::Stopping,
                    _ => InstanceStatus::Running,
                }
            } else {
                InstanceStatus::Exited
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::{
        DnsMode, DnsProfile, LaunchTarget, LocaleProfile, RegistryProfile, TimezoneProfile,
    };

    fn sample_app(profile_id: Uuid) -> Application {
        Application {
            id: Uuid::new_v4(),
            name: "Test".into(),
            launch: LaunchTarget::Command {
                command: "cmd".into(),
            },
            console_host: envbox_core::ConsoleHost::Direct,
            working_directory: None,
            arguments: vec!["/c".into(), "exit 0".into()],
            default_profile_id: profile_id,
            inherit_children: true,
            audit: false,
        }
    }

    fn sample_profile() -> EnvironmentProfile {
        EnvironmentProfile {
            id: Uuid::new_v4(),
            name: "US".into(),
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
            environment: HashMap::new(),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        }
    }

    #[test]
    fn status_from_stats_running_while_children_live() {
        let live = JobStats {
            active_processes: 2,
            ..Default::default()
        };
        assert_eq!(
            status_from_stats(&live, InstanceStatus::Running),
            InstanceStatus::Running
        );
        let empty = JobStats::default();
        assert_eq!(
            status_from_stats(&empty, InstanceStatus::Running),
            InstanceStatus::Exited
        );
    }

    #[test]
    fn status_from_stats_fast_exit_root_is_exited_not_failed() {
        let empty = JobStats::default();
        assert_eq!(
            status_from_stats(&empty, InstanceStatus::Starting),
            InstanceStatus::Exited
        );
        let live = JobStats {
            active_processes: 1,
            ..Default::default()
        };
        assert_eq!(
            status_from_stats(&live, InstanceStatus::Starting),
            InstanceStatus::Running
        );
    }

    #[test]
    fn status_from_stats_stopping_resolves_to_exited_when_empty() {
        let empty = JobStats::default();
        assert_eq!(
            status_from_stats(&empty, InstanceStatus::Stopping),
            InstanceStatus::Exited
        );
        let live = JobStats {
            active_processes: 1,
            ..Default::default()
        };
        assert_eq!(
            status_from_stats(&live, InstanceStatus::Stopping),
            InstanceStatus::Stopping
        );
    }

    #[test]
    fn children_exclude_root_process() {
        assert_eq!(children_from_active_processes(0), 0);
        assert_eq!(children_from_active_processes(1), 0);
        assert_eq!(children_from_active_processes(3), 2);
    }

    #[test]
    fn run_with_does_not_mutate_default_profile_id() {
        let profile = sample_profile();
        let app = sample_app(profile.id);
        let default_before = app.default_profile_id;
        let mut mgr = InstanceManager::new();
        // Launch may fail without runtime DLL in unit tests; contract is no mutation.
        let _ = mgr.run(&app, RunTarget::Profile(profile));
        assert_eq!(app.default_profile_id, default_before);
    }

    #[test]
    fn host_target_builds_unvirtualized_session_request() {
        let app = sample_app(Uuid::new_v4());
        let req = build_session_request(&app, RunTarget::Host).unwrap();
        assert!(req.profile.is_none(), "Host must not carry a Profile");
        assert_eq!(req.arguments, app.arguments);
    }

    #[test]
    fn profile_target_carries_profile_on_session_request() {
        let profile = sample_profile();
        let profile_id = profile.id;
        let app = sample_app(profile_id);
        let req = build_session_request(&app, RunTarget::Profile(profile)).unwrap();
        let p = req.profile.expect("Profile target must inject a Profile");
        assert_eq!(p.id, profile_id);
        assert_ne!(p.id, Uuid::nil());
    }

    #[test]
    fn run_target_host_profile_id_is_nil_not_a_fake_profile() {
        assert_eq!(RunTarget::Host.profile_id(), Uuid::nil());
        let p = sample_profile();
        let id = p.id;
        assert_eq!(RunTarget::Profile(p).profile_id(), id);
    }

    #[test]
    fn aumid_executable_normalizes_to_packaged() {
        let app = Application {
            id: Uuid::new_v4(),
            name: "ChatGPT".into(),
            launch: LaunchTarget::Executable {
                path: "shell:AppsFolder\\OpenAi.Codex_2p2nqsd0c76g0!App".into(),
            },
            console_host: envbox_core::ConsoleHost::Direct,
            working_directory: None,
            arguments: vec![],
            default_profile_id: Uuid::nil(),
            inherit_children: true,
            audit: false,
        };
        let req = build_session_request(&app, RunTarget::Host).unwrap();
        match req.launch {
            LaunchTarget::Packaged {
                aumid,
                package_family_name,
                ..
            } => {
                assert_eq!(aumid, "OpenAi.Codex_2p2nqsd0c76g0!App");
                assert_eq!(package_family_name, "OpenAi.Codex_2p2nqsd0c76g0");
            }
            other => panic!("expected Packaged, got {other:?}"),
        }
    }

    #[test]
    fn cmd_console_host_wraps_command_without_losing_arguments() {
        let mut app = sample_app(Uuid::new_v4());
        app.console_host = ConsoleHost::Cmd;
        app.arguments = vec!["--name".into(), "two words".into()];
        let (launch, args) = console_host_target(&app).unwrap();
        assert_eq!(
            launch,
            LaunchTarget::Command {
                command: "cmd.exe".into()
            }
        );
        assert_eq!(&args[..4], ["/d", "/v:off", "/s", "/c"]);
        assert!(args[4].contains("two words"));
    }

    #[test]
    fn powershell_console_host_uses_interactive_command_and_exit_code() {
        let mut app = sample_app(Uuid::new_v4());
        app.console_host = ConsoleHost::PowerShell;
        let (launch, args) = console_host_target(&app).unwrap();
        assert_eq!(
            launch,
            LaunchTarget::Command {
                command: "powershell.exe".into()
            }
        );
        assert_eq!(args[0], "-NoLogo");
        assert_eq!(args[1], "-NoProfile");
        assert_eq!(args[2], "-Command");
        assert!(args[3].contains("$c.ExitCode"));
    }

    #[test]
    fn shell_host_resolves_cmd_wrapper_before_same_named_ps1() {
        let dir = std::env::temp_dir().join(format!("envbox-shell-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("my-claude.ps1"), "Write-Error wrong-shim").unwrap();
        std::fs::write(dir.join("my-claude.cmd"), "@echo off\r\n").unwrap();

        let mut app = sample_app(Uuid::new_v4());
        app.launch = LaunchTarget::Command {
            command: "my-claude".into(),
        };
        app.console_host = ConsoleHost::PowerShell;
        let path = dir.to_string_lossy().into_owned();
        let (_, args) = console_host_target_with_path(&app, Some(&path)).unwrap();
        assert!(args[3].contains("my-claude.cmd"), "args={args:?}");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn direct_console_host_preserves_original_target() {
        let app = sample_app(Uuid::new_v4());
        let (launch, args) = console_host_target(&app).unwrap();
        assert_eq!(launch, app.launch);
        assert_eq!(args, app.arguments);
    }

    #[test]
    fn cmd_console_host_rejects_percent_expansion_arguments() {
        let mut app = sample_app(Uuid::new_v4());
        app.console_host = ConsoleHost::Cmd;
        app.arguments = vec!["100%PATH%".into()];
        let err = console_host_target(&app).unwrap_err();
        assert!(matches!(err, InstanceError::InvalidConsoleHost(message) if message.contains("%")));
    }

    #[test]
    fn windows_terminal_args_do_not_include_user_command_or_arguments() {
        let id = Uuid::new_v4();
        let args = windows_terminal_args(
            Path::new(r"C:\Program Files\Aura\envbox.exe"),
            Path::new(r"C:\Users\test"),
            Path::new(r"C:\Users\test\AppData\Local\com.aura.envbox"),
            123,
            id,
            Uuid::new_v4(),
            Uuid::nil(),
            &format!("Local\\Aura-{id}"),
        );
        assert!(args.contains(&"hidden".into()));
        assert!(args.contains(&"terminal-run".into()));
        assert!(!args.iter().any(|arg| arg == "cmd /c user-command"));
    }

    /// Opens short-lived visible console windows; run explicitly in an
    /// interactive Windows session to verify the GUI shell launch path.
    #[cfg(windows)]
    #[test]
    #[ignore = "opens visible Cmd and PowerShell consoles"]
    fn selected_shells_give_node_a_real_console() {
        use std::time::Instant;

        let base = std::env::temp_dir().join(format!("envbox-console-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let script = base.join("check.js");
        std::fs::write(
            &script,
            "require('fs').writeFileSync(process.argv[2], String(!!process.stdin.isTTY) + ',' + String(!!process.stdout.isTTY) + ';' + process.cwd() + ';' + process.argv.slice(3).join('|'));",
        )
        .unwrap();
        let home = crate::launcher::effective_working_directory(
            &LaunchTarget::Command {
                command: "node.exe".into(),
            },
            None,
        )
        .unwrap()
        .unwrap();
        let expected = format!("true,true;{};C:\\dir with space\\|next", home.display());

        for host in [ConsoleHost::Cmd, ConsoleHost::PowerShell] {
            let output = base.join(format!("{host}.txt"));
            let mut app = sample_app(Uuid::nil());
            app.launch = LaunchTarget::Command {
                command: "node.exe".into(),
            };
            app.console_host = host;
            app.arguments = vec![
                script.display().to_string(),
                output.display().to_string(),
                "C:\\dir with space\\".into(),
                "next".into(),
            ];
            let mut manager = InstanceManager::new();
            let id = manager.run(&app, RunTarget::Host).expect("start shell");
            let deadline = Instant::now() + Duration::from_secs(10);
            while !output.is_file() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            let result = std::fs::read_to_string(&output).expect("node wrote console state");
            assert_eq!(
                result, expected,
                "{host} changed the console, cwd, or arguments"
            );
            let _ = manager.stop(id);
        }
        let wrapper = base.join("node wrapper.cmd");
        std::fs::write(
            &wrapper,
            format!("@echo off\r\nnode.exe \"{}\" %*\r\n", script.display()),
        )
        .unwrap();
        for host in [ConsoleHost::Cmd, ConsoleHost::PowerShell] {
            let output = base.join(format!("{host}-batch.txt"));
            let mut app = sample_app(Uuid::nil());
            app.launch = LaunchTarget::Command {
                command: wrapper.display().to_string(),
            };
            app.console_host = host;
            app.arguments = vec![
                output.display().to_string(),
                "C:\\dir with space\\".into(),
                "next".into(),
            ];
            let mut manager = InstanceManager::new();
            let id = manager
                .run(&app, RunTarget::Host)
                .expect("start batch wrapper");
            let deadline = Instant::now() + Duration::from_secs(10);
            while !output.is_file() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
            assert_eq!(
                std::fs::read_to_string(&output).expect("batch wrapper wrote console state"),
                expected,
                "{host} changed batch wrapper arguments"
            );
            let _ = manager.stop(id);
            let _ = std::fs::remove_file(output);
        }
        let _ = std::fs::remove_file(wrapper);
        let _ = std::fs::remove_file(script);
        for host in [ConsoleHost::Cmd, ConsoleHost::PowerShell] {
            let _ = std::fs::remove_file(base.join(format!("{host}.txt")));
        }
        let _ = std::fs::remove_dir(base);
    }
}
