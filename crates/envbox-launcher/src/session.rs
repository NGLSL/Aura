//! EnvironmentSession start pipeline (packaged-v1 ticket 37/41).
//!
//! Order: resolve → capability probe → backend selection → activation →
//! attach → bootstrap → track. Win32 golden path stays PreExecution.

use crate::activation::{
    backend_for, ActivateError, ActivationRequest, ActivatedTarget,
};
use crate::attach::{select_strategy, AttachError, AttachedRuntime, RuntimeInjector};
use crate::capability::win32_capabilities;
use crate::environment::build_environment_block;
use crate::job::{InstanceJob, JobError};
use crate::launcher::{LaunchError, LaunchedProcess};
use crate::ipc::SessionTable;
use envbox_core::{
    isolation_for_strategy, AttachStrategy, EnvironmentProfile, EnvironmentSession,
    IsolationGuarantee, LaunchTarget, RuntimeInstance, SessionState,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Activate(#[from] ActivateError),
    #[error(transparent)]
    Attach(#[from] AttachError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error("unsupported capability: {0}")]
    Unsupported(String),
    #[error("profile invalid: {0}")]
    InvalidProfile(String),
    #[error("injection failed — Startup Fail Policy (no silent unvirtualized launch)")]
    InjectionFailed,
}

/// One Environment Session (control-plane aggregate + runtime handles).
pub struct SessionHandle {
    pub session: EnvironmentSession,
    pub instance: RuntimeInstance,
    pub job: Option<InstanceJob>,
    pub attached: Option<AttachedRuntime>,
    pub child: Option<LaunchedProcess>,
    /// IPC table entry (profile payload) for this session.
    pub profile_payload: Option<String>,
    /// Host Named Pipe broker (packaged / IPC bootstrap).
    pub broker: Option<crate::ipc_server::HostBroker>,
    #[cfg(windows)]
    process: Option<crate::launcher::win::SafeHandle>,
    /// Primary thread handle kept for lifetime / resume bookkeeping.
    #[cfg(windows)]
    #[allow(dead_code)]
    thread: Option<crate::launcher::win::SafeHandle>,
}

impl SessionHandle {
    /// Wait for the root process and return its exit code.
    pub fn wait_root(&mut self) -> Result<i32, SessionError> {
        if let Some(child) = &mut self.child {
            let status = child
                .wait()
                .map_err(|e| SessionError::Unsupported(format!("wait failed: {e}")))?;
            return Ok(status.code().unwrap_or(1));
        }
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
            let Some(process) = &self.process else {
                return Ok(0);
            };
            unsafe {
                if WaitForSingleObject(process.0, INFINITE).0 == 0xFFFF_FFFF {
                    return Err(SessionError::Unsupported("WaitForSingleObject failed".into()));
                }
                let mut code = 0u32;
                if GetExitCodeProcess(process.0, &mut code).is_err() {
                    return Err(SessionError::Unsupported("GetExitCodeProcess failed".into()));
                }
                Ok(code as i32)
            }
        }
        #[cfg(not(windows))]
        {
            Ok(0)
        }
    }
}

/// Start request for one Run (CLI/GUI share this seam).
#[derive(Debug, Clone)]
pub struct SessionStartRequest {
    pub application_id: Uuid,
    pub launch: LaunchTarget,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// `Some` = Environment Profile (virtualize). `None` = Host (no injection).
    pub profile: Option<EnvironmentProfile>,
    pub inherit_children: bool,
    pub audit: bool,
}

/// Start an Environment Session.
///
/// Host (no profile): plain create, no Runtime injection.
/// Profile: activate → attach → resume (Win32) or activate → attach (Packaged).
pub fn start_session(req: SessionStartRequest) -> Result<SessionHandle, SessionError> {
    let instance_id = Uuid::new_v4();
    let profile_id = req.profile.as_ref().map(|p| p.id).unwrap_or_default();
    let host_mode = req.profile.is_none();

    if let Some(profile) = &req.profile {
        profile
            .validate()
            .map_err(|e| SessionError::InvalidProfile(e.to_string()))?;
    }

    let host_env: HashMap<String, String> = std::env::vars().collect();
    let pipe_path = crate::ipc_server::session_pipe_name(&instance_id.to_string());
    let env = if host_mode {
        host_env
    } else {
        let mut env = build_environment_block(
            &host_env,
            req.profile.as_ref(),
            instance_id,
            profile_id,
            req.inherit_children,
            req.audit,
        );
        // Point Runtime IPC Bootstrap at this session's Host pipe.
        env.insert("ENVBOX_IPC_PIPE".into(), pipe_path.clone());
        env
    };

    let is_packaged = matches!(req.launch, LaunchTarget::Packaged { .. });

    // Capability snapshot before activation (Win32 we create = full caps).
    let pre_caps = if is_packaged {
        // Packaged: cannot suspend, cannot use environment block.
        envbox_core::TargetCapabilities {
            can_suspend: false,
            can_inject_runtime: true, // refined after probe
            can_create_environment_block: false,
            can_assign_job: true,
            can_track_children: true,
        }
    } else {
        win32_capabilities()
    };

    let strategy = if host_mode {
        AttachStrategy::PreExecution
    } else {
        select_strategy(&pre_caps).ok_or_else(|| {
            SessionError::Unsupported("no attach strategy for target capabilities".into())
        })?
    };

    let isolation = if host_mode {
        IsolationGuarantee::Partial
    } else {
        isolation_for_strategy(strategy)
    };

    let mut session = EnvironmentSession::new(
        req.application_id,
        req.launch.clone(),
        profile_id,
        isolation,
        strategy,
    );
    session.id = instance_id;

    // Activation
    let runtime_dll = if host_mode {
        None
    } else {
        Some(
            crate::injection::resolve_runtime_dll().map_err(|e| {
                SessionError::Activate(ActivateError::Inject(e))
            })?,
        )
    };

    let activation_req = ActivationRequest {
        arguments: req.arguments.clone(),
        working_directory: req.working_directory.clone(),
        environment: env.clone(),
        runtime_dll: runtime_dll.clone(),
        require_runtime: !host_mode,
    };

    let backend = backend_for(&req.launch);
    let activated: ActivatedTarget = backend.activate(&req.launch, &activation_req)?;

    // Fail closed when Runtime is required but injection is unsupported.
    if !host_mode && !activated.injection_supported {
        return Err(SessionError::Unsupported(
            activated
                .injection_reason
                .clone()
                .unwrap_or_else(|| "runtime injection unsupported".into()),
        ));
    }

    session.register_root(activated.pid);
    if let Some(ident) = activated.package_identity.clone() {
        session.package_identity = Some(ident);
    }
    session.state = SessionState::Activated;

    // Job (lifecycle only; best-effort for packaged).
    let mut job = InstanceJob::create()?;
    let _job_ok = job.assign_pid(activated.pid).is_ok();

    // Attach
    let attached = if host_mode {
        None
    } else {
        let dll = runtime_dll
            .clone()
            .ok_or(SessionError::InjectionFailed)?;
        let injector = RuntimeInjector::new(dll);
        let already = activated.suspended && !is_packaged; // Detours at create
        match injector.attach(activated.pid, strategy, already) {
            Ok(a) => {
                session.state = SessionState::Attached;
                Some(a)
            }
            Err(err) => {
                // Startup Fail Policy: never leave unvirtualized process as success.
                let _ = job.terminate();
                return Err(err.into());
            }
        }
    };

    // Resume Win32 suspended root after attach.
    if activated.suspended {
        #[cfg(windows)]
        {
            crate::launcher::resume_activated(&activated)?;
        }
    }

    session.state = SessionState::Running;

    // IPC bootstrap table + Named Pipe host (packaged roots have no ENVBOX_*).
    let mut table = SessionTable::new();
    if let Some(profile) = &req.profile {
        table.set_instance_id(&instance_id.to_string());
        table.register_profile(profile);
        table.bind_pid(activated.pid, &profile_id.to_string());
    }
    let shared: crate::ipc_server::SharedTable = std::sync::Arc::new(std::sync::Mutex::new(table));
    let broker = match crate::ipc_server::HostBroker::start_on(shared, pipe_path) {
        Ok(b) => Some(b),
        Err(_) => None, // Fail open: Win32 ENVBOX_* fallback still works.
    };

    let profile_payload = req
        .profile
        .as_ref()
        .map(|p| crate::ipc::profile_to_message(p, &instance_id.to_string()).encode_line());

    let mut process_ids = HashSet::new();
    process_ids.insert(activated.pid);
    let package_family_name = session
        .package_identity
        .as_ref()
        .map(|p| p.package_family_name.clone());
    let aumid = session
        .package_identity
        .as_ref()
        .map(|p| p.aumid.clone());
    let instance = RuntimeInstance {
        id: instance_id,
        application_id: req.application_id,
        profile_id,
        root_pid: activated.pid,
        process_ids,
        started_at: std::time::SystemTime::now(),
        // Job assign is lifecycle-only (not an isolation boundary). Record it
        // on the handle; instance still starts if assign failed.
        status: envbox_core::InstanceStatus::Running,
        package_family_name,
        aumid,
        isolation_guarantee: Some(isolation),
        attach_strategy: Some(strategy),
    };

    #[cfg(windows)]
    let (root_process, root_thread) = {
        let ActivatedTarget {
            process, thread, ..
        } = activated;
        (Some(process), thread)
    };

    Ok(SessionHandle {
        session,
        instance,
        job: Some(job),
        attached,
        child: None,
        profile_payload,
        broker,
        #[cfg(windows)]
        process: root_process,
        #[cfg(windows)]
        thread: root_thread,
    })
}

/// Register a child process with the session (unified Win32/Packaged).
pub fn register_child(session: &mut EnvironmentSession, pid: u32) {
    session.register_child(pid);
}

/// Membership: root descendant OR package family + activation window.
pub fn belongs(
    session: &EnvironmentSession,
    pid: u32,
    is_descendant: bool,
    package_family: Option<&str>,
    created_in_window: bool,
) -> bool {
    session.belongs_to_session(pid, is_descendant, package_family, created_in_window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::{
        DnsMode, DnsProfile, LocaleProfile, RegistryProfile, TimezoneProfile,
    };

    fn profile() -> EnvironmentProfile {
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
        }
    }

    #[test]
    fn host_session_target_has_no_profile() {
        let req = SessionStartRequest {
            application_id: Uuid::nil(),
            launch: LaunchTarget::Command {
                command: "cmd".into(),
            },
            arguments: vec!["/c".into(), "exit 0".into()],
            working_directory: None,
            profile: None,
            inherit_children: true,
            audit: false,
        };
        // May fail without runtime DLL for profile mode; host mode should reach activate.
        // We only assert request shape here.
        assert!(req.profile.is_none());
        assert!(!matches!(req.launch, LaunchTarget::Packaged { .. }));
    }

    #[test]
    fn packaged_target_selects_post_activation_shape() {
        let mut caps = envbox_core::TargetCapabilities::default();
        caps.can_suspend = false;
        caps.can_inject_runtime = true;
        let s = select_strategy(&caps).unwrap();
        assert_eq!(s, AttachStrategy::PostActivation);
    }

    #[test]
    fn package_membership_requires_window() {
        use envbox_core::PackageIdentity;
        let mut s = EnvironmentSession::new(
            Uuid::nil(),
            LaunchTarget::Packaged {
                aumid: "Foo!App".into(),
                package_full_name: "Foo_1.0_x64__abc".into(),
                package_family_name: "Foo_abc".into(),
            },
            Uuid::nil(),
            IsolationGuarantee::PostActivation,
            AttachStrategy::PostActivation,
        );
        s.package_identity = Some(PackageIdentity {
            aumid: "Foo!App".into(),
            package_full_name: "Foo_1.0_x64__abc".into(),
            package_family_name: "Foo_abc".into(),
        });
        assert!(belongs(&s, 1, true, None, false));
        assert!(belongs(&s, 2, false, Some("Foo_abc"), true));
        assert!(!belongs(&s, 3, false, Some("Foo_abc"), false));
    }

    #[test]
    fn profile_message_encodes_from_profile() {
        let p = profile();
        let msg = crate::ipc::profile_to_message(&p, "inst-1");
        let line = msg.encode_line();
        assert!(line.contains("locale_name=en-US"));
        assert!(line.contains("region=US"));
    }
}
