//! Activation backends (packaged-v1 ticket 37/39).
//!
//! Activation and attach are separate: activate produces a live PID, attach
//! injects the Runtime. No `if windowsapps` outside backend selection.

use envbox_core::{LaunchTarget, PackageIdentity, TargetCapabilities};
use thiserror::Error;

use crate::capability::{capabilities_after_probe, probe_pid, win32_capabilities};

#[derive(Debug, Error)]
pub enum ActivateError {
    #[error("target not activatable: {0}")]
    UnsupportedTarget(String),
    #[error("cannot resolve executable: {0}")]
    Resolve(String),
    #[error("CreateProcess failed (GetLastError={0})")]
    CreateProcess(u32),
    #[error("OpenProcess failed (GetLastError={0})")]
    OpenProcess(u32),
    #[error("ResumeThread failed (GetLastError={0})")]
    ResumeThread(u32),
    #[error("AUMID activation failed: {0}")]
    AumidActivate(String),
    #[error("activated but pid is 0")]
    EmptyPid,
    #[error("working directory does not exist: {0}")]
    WorkingDirectoryMissing(std::path::PathBuf),
    #[error("profile invalid: {0}")]
    InvalidProfile(String),
    #[error(transparent)]
    Inject(#[from] crate::injection::InjectError),
}

/// Process as activated (exists; may be suspended).
pub struct ActivatedTarget {
    pub pid: u32,
    pub package_identity: Option<PackageIdentity>,
    /// True when the primary thread is still suspended (PreExecution).
    pub suspended: bool,
    pub capabilities: TargetCapabilities,
    pub injection_supported: bool,
    pub injection_reason: Option<String>,
    #[cfg(windows)]
    pub process: crate::launcher::win::SafeHandle,
    #[cfg(windows)]
    pub thread: Option<crate::launcher::win::SafeHandle>,
}

/// What an activation backend needs to start a target.
#[derive(Debug, Clone)]
pub struct ActivationRequest<'a> {
    /// Ordinary Win32 roots join this Job atomically at process creation.
    pub creation_job: Option<crate::job::JobAssignment<'a>>,
    pub arguments: Vec<String>,
    pub working_directory: Option<std::path::PathBuf>,
    /// Merged environment (including ENVBOX_* for Win32 fallback).
    pub environment: std::collections::HashMap<String, String>,
    /// Runtime DLL to inject at create time (Win32 PreExecution). `None` = plain create.
    pub runtime_dll: Option<std::path::PathBuf>,
    /// Startup Fail Policy: never launch unvirtualized when a profile is set.
    pub require_runtime: bool,
    /// Browser / Network Guard: root-process Chromium switch (Win32 only).
    pub webrtc_policy: Option<envbox_core::WebRtcPolicy>,
    /// Profile locale for Chromium's process-level Intl and language switches.
    pub browser_locale: Option<String>,
    /// GUI-selected Cmd/PowerShell roots need their own interactive console.
    /// Direct CLI runs keep the caller's existing console.
    pub create_new_console: bool,
}

/// ActivationBackend: produce a live process. Attach is a separate seam.
pub trait ActivationBackend {
    fn activate(
        &self,
        target: &LaunchTarget,
        req: &ActivationRequest,
    ) -> Result<ActivatedTarget, ActivateError>;
}

/// Win32 / Command backend: CreateProcess(CREATE_SUSPENDED) [+ Detours inject].
pub struct Win32ActivationBackend;

/// Apply the same working-directory policy used by the direct launch path.
///
/// This helper runs after the target kind is known, which is necessary to
/// distinguish a CLI `Command` from a GUI `Executable` before CreateProcess.
fn effective_activation_request<'a>(
    target: &LaunchTarget,
    req: &ActivationRequest<'a>,
) -> Result<ActivationRequest<'a>, ActivateError> {
    let working_directory =
        crate::launcher::effective_working_directory(target, req.working_directory.as_deref())
            .map_err(|error| match error {
                crate::launcher::LaunchError::WorkingDirectoryMissing(path) => {
                    ActivateError::WorkingDirectoryMissing(path)
                }
                other => ActivateError::Resolve(other.to_string()),
            })?;

    if let Some(dir) = &working_directory {
        if !dir.is_dir() {
            return Err(ActivateError::WorkingDirectoryMissing(dir.clone()));
        }
    }

    let mut effective = req.clone();
    effective.working_directory = working_directory;
    Ok(effective)
}

impl ActivationBackend for Win32ActivationBackend {
    fn activate(
        &self,
        target: &LaunchTarget,
        req: &ActivationRequest,
    ) -> Result<ActivatedTarget, ActivateError> {
        let (resolved, user_args) = match target {
            LaunchTarget::Executable { path } => {
                let s = path.to_string_lossy();
                if crate::package_discovery::is_windows_apps_path(&s) {
                    return Err(ActivateError::UnsupportedTarget(format!(
                        "refusing CreateProcess of WindowsApps path (would drop package identity): {s}; \
                         use AUMID / LaunchTarget::Packaged"
                    )));
                }
                if !path.is_file() {
                    return Err(ActivateError::Resolve(path.display().to_string()));
                }
                (
                    crate::command::ResolvedCommand {
                        program: path.clone(),
                        via_comspec: false,
                        comspec_payload: None,
                    },
                    req.arguments.clone(),
                )
            }
            LaunchTarget::Command { command } => {
                if crate::package_discovery::is_windows_apps_path(command)
                    || crate::package_discovery::extract_aumid(command).is_some()
                {
                    return Err(ActivateError::UnsupportedTarget(format!(
                        "refusing CreateProcess of packaged target {command:?}; \
                         use AUMID / LaunchTarget::Packaged"
                    )));
                }
                let path_env = req
                    .environment
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("path"))
                    .map(|(_, v)| v.clone());
                let resolved = crate::command::resolve_command(command, path_env.as_deref())
                    .map_err(|e| ActivateError::Resolve(e.to_string()))?;
                // Keep resolve_command's ComSpec payload (original token); do not
                // rewrite it to a full path with spaces.
                (resolved, req.arguments.clone())
            }
            LaunchTarget::Packaged { .. } => {
                return Err(ActivateError::UnsupportedTarget(
                    "Packaged target requires PackagedActivationBackend".into(),
                ));
            }
        };

        let effective_req = effective_activation_request(target, req)?;

        let spawn = crate::launcher::spawn_for_activation(&resolved, &user_args, &effective_req)?;
        let (injection_supported, injection_reason) = if effective_req.require_runtime {
            if effective_req.runtime_dll.is_some() {
                (true, None)
            } else {
                (
                    false,
                    Some("runtime DLL missing (Startup Fail Policy)".into()),
                )
            }
        } else {
            (effective_req.runtime_dll.is_some(), None)
        };

        Ok(ActivatedTarget {
            pid: spawn.pid,
            package_identity: None,
            suspended: spawn.suspended,
            capabilities: win32_capabilities(),
            injection_supported,
            injection_reason,
            #[cfg(windows)]
            process: spawn.process,
            #[cfg(windows)]
            thread: spawn.thread,
        })
    }
}

/// Packaged backend: AUMID activation only — never a raw WindowsApps exe.
pub struct PackagedActivationBackend;

impl ActivationBackend for PackagedActivationBackend {
    fn activate(
        &self,
        target: &LaunchTarget,
        _req: &ActivationRequest,
    ) -> Result<ActivatedTarget, ActivateError> {
        let (aumid, package_full_name, package_family_name) = match target {
            LaunchTarget::Packaged {
                aumid,
                package_full_name,
                package_family_name,
            } => (
                aumid.clone(),
                package_full_name.clone(),
                package_family_name.clone(),
            ),
            _ => {
                return Err(ActivateError::UnsupportedTarget(
                    "PackagedActivationBackend requires LaunchTarget::Packaged".into(),
                ));
            }
        };

        let (pid, activation_started) = activate_aumid(&aumid)?;
        if pid == 0 {
            return Err(ActivateError::EmptyPid);
        }

        #[cfg(windows)]
        if !fresh_activation_generation(
            activation_started,
            crate::ipc_server::process_creation_time(pid),
        ) {
            // ActivateApplication can deliver to an existing singleton. This
            // rejection intentionally precedes attach, Job assignment or Stop.
            return Err(ActivateError::UnsupportedTarget(
                "packaged activation returned an existing or unverified process; refusing new-instance ownership".into(),
            ));
        }
        #[cfg(not(windows))]
        let _ = activation_started;

        let probe = probe_pid(pid);
        let injection = probe.to_injection_capability(true);
        if injection.is_app_container {
            // Fail closed: AppContainer never reaches a success path.
            return Err(ActivateError::UnsupportedTarget(
                injection
                    .reason
                    .unwrap_or_else(|| "AppContainer target unsupported".into()),
            ));
        }

        let package_identity = Some(PackageIdentity {
            aumid,
            package_full_name,
            package_family_name,
        });

        Ok(ActivatedTarget {
            pid,
            package_identity,
            suspended: false,
            capabilities: capabilities_after_probe(&probe, injection.supported),
            injection_supported: injection.supported,
            injection_reason: injection.reason,
            #[cfg(windows)]
            process: crate::launcher::open_process_handle(pid)?,
            #[cfg(windows)]
            thread: None,
        })
    }
}

/// `IApplicationActivationManager::ActivateApplication(AUMID)` → PID.
fn fresh_activation_generation(started: u64, created: Option<u64>) -> bool {
    started != 0 && created.is_some_and(|created| created >= started)
}

fn activate_aumid(aumid: &str) -> Result<(u32, u64), ActivateError> {
    #[cfg(windows)]
    {
        win_activate_aumid(aumid)
    }
    #[cfg(not(windows))]
    {
        let _ = aumid;
        Err(ActivateError::UnsupportedTarget(
            "AUMID activation is Windows-only".into(),
        ))
    }
}

#[cfg(windows)]
fn win_activate_aumid(aumid: &str) -> Result<(u32, u64), ActivateError> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ApplicationActivationManager, IApplicationActivationManager, AO_NONE,
    };

    unsafe {
        // Best-effort COM init; already-initialized is fine.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let manager: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_ALL)
                .map_err(|e| ActivateError::AumidActivate(format!("CoCreateInstance: {e}")))?;

        let aumid_h = HSTRING::from(aumid);
        let args = HSTRING::new();
        let now = windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime();
        let started = (u64::from(now.dwHighDateTime) << 32) | u64::from(now.dwLowDateTime);
        let pid = manager
            .ActivateApplication(&aumid_h, &args, AO_NONE)
            .map_err(|e| ActivateError::AumidActivate(format!("ActivateApplication: {e}")))?;
        Ok((pid, started))
    }
}

/// Choose activation backend from the launch target (path/type at the seam).
pub fn backend_for(target: &LaunchTarget) -> Box<dyn ActivationBackend> {
    match target {
        LaunchTarget::Packaged { .. } => Box::new(PackagedActivationBackend),
        LaunchTarget::Executable { .. } | LaunchTarget::Command { .. } => {
            Box::new(Win32ActivationBackend)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn packaged_freshness_rejects_existing_and_unknown_generation() {
        assert!(!fresh_activation_generation(100, Some(99)));
        assert!(!fresh_activation_generation(100, None));
        assert!(!fresh_activation_generation(0, Some(100)));
        assert!(fresh_activation_generation(100, Some(100)));
        assert!(fresh_activation_generation(100, Some(101)));
    }
    use std::path::PathBuf;

    fn request(working_directory: Option<PathBuf>) -> ActivationRequest<'static> {
        ActivationRequest {
            creation_job: None,
            arguments: Vec::new(),
            working_directory,
            environment: HashMap::new(),
            runtime_dll: None,
            require_runtime: false,
            webrtc_policy: None,
            browser_locale: None,
            create_new_console: false,
        }
    }

    #[test]
    fn command_activation_defaults_to_user_profile_directory() {
        let target = LaunchTarget::Command {
            command: "my-cli".into(),
        };
        let req = request(None);
        let effective = effective_activation_request(&target, &req).unwrap();
        let expected = crate::launcher::effective_working_directory(&target, None).unwrap();
        assert_eq!(effective.working_directory, expected);
        assert!(effective.working_directory.is_some());
    }

    #[test]
    fn executable_activation_keeps_inherited_working_directory() {
        let target = LaunchTarget::Executable {
            path: PathBuf::from(r"C:\Program Files\my-gui.exe"),
        };
        let effective = effective_activation_request(&target, &request(None)).unwrap();
        assert_eq!(effective.working_directory, None);
    }

    #[test]
    fn activation_explicit_working_directory_wins_for_commands() {
        let target = LaunchTarget::Command {
            command: "my-cli".into(),
        };
        let explicit = std::env::current_dir().unwrap();
        let effective =
            effective_activation_request(&target, &request(Some(explicit.clone()))).unwrap();
        assert_eq!(effective.working_directory, Some(explicit));
    }
}
