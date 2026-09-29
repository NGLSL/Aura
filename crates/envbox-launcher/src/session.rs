//! EnvironmentSession start pipeline (packaged-v1 ticket 37/41).
//!
//! Order: resolve → capability probe → backend selection → activation →
//! attach → bootstrap → track. Win32 golden path stays PreExecution.

use crate::activation::{backend_for, ActivateError, ActivatedTarget, ActivationRequest};
use crate::attach::{select_strategy, AttachError, AttachedRuntime, RuntimeInjector};
use crate::capability::win32_capabilities;
use crate::environment::build_environment_block;
use crate::ipc::SessionTable;
use crate::job::{InstanceJob, JobError};
use crate::launcher::{LaunchError, LaunchedProcess};
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
    /// Process Tracker (Job + Package/PID dual backend).
    pub tracker: crate::process_tracker::ProcessTracker,
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
            use windows::Win32::System::Threading::{
                GetExitCodeProcess, WaitForSingleObject, INFINITE,
            };
            let Some(process) = &self.process else {
                return Ok(0);
            };
            unsafe {
                if WaitForSingleObject(process.0, INFINITE).0 == 0xFFFF_FFFF {
                    return Err(SessionError::Unsupported(
                        "WaitForSingleObject failed".into(),
                    ));
                }
                let mut code = 0u32;
                if GetExitCodeProcess(process.0, &mut code).is_err() {
                    return Err(SessionError::Unsupported(
                        "GetExitCodeProcess failed".into(),
                    ));
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

/// Small local handoff marker used only between the GUI and the hidden
/// Windows Terminal bridge. It carries the exact root PID because Job Object
/// process-list order is unspecified. The GUI removes stale markers when a
/// handoff times out; the generated UUID keeps concurrent launches isolated.
pub fn terminal_root_marker_path(instance_id: Uuid) -> PathBuf {
    std::env::temp_dir().join(format!("envbox-terminal-{instance_id}.root"))
}

/// Non-overwritable cancellation marker for a Windows Terminal handoff.
/// Keeping this separate from the root-PID state prevents terminal-run from
/// replacing a Stop request while publishing its `running` marker.
pub fn terminal_cancel_marker_path(instance_id: Uuid) -> PathBuf {
    std::env::temp_dir().join(format!("envbox-terminal-{instance_id}.cancel"))
}

/// Start an Environment Session.
///
/// Host (no profile): plain create, no Runtime injection.
/// Profile: activate → attach → resume (Win32) or activate → attach (Packaged).
pub fn start_session(req: SessionStartRequest) -> Result<SessionHandle, SessionError> {
    start_session_with_options(req, None, None, false)
}

/// Start a GUI-selected shell with its own interactive console. This is only
/// used for Cmd/PowerShell Application preferences; `envbox run` keeps the
/// caller's console, and Windows Terminal supplies a ConPTY itself.
pub fn start_session_in_new_console(
    req: SessionStartRequest,
) -> Result<SessionHandle, SessionError> {
    start_session_with_options(req, None, None, true)
}

/// Start a session in a Job Object that was created by another process.
///
/// This is the handoff seam for Windows Terminal: the GUI creates a unique
/// named Job, launches `wt.exe`, and the hidden `terminal-run` command opens
/// the name before starting the real CLI. The supplied instance ID is kept so
/// the Runtime and the GUI refer to the same instance.
pub fn start_session_in_named_job(
    req: SessionStartRequest,
    instance_id: Uuid,
    job_name: impl AsRef<str>,
) -> Result<SessionHandle, SessionError> {
    start_session_with_options(
        req,
        Some(instance_id),
        Some(job_name.as_ref().to_string()),
        false,
    )
}

fn start_session_with_options(
    req: SessionStartRequest,
    requested_instance_id: Option<Uuid>,
    requested_job_name: Option<String>,
    create_new_console: bool,
) -> Result<SessionHandle, SessionError> {
    // Safety net: every caller (GUI/CLI/tests) gets Packaged for AUMID /
    // WindowsApps targets. Never CreateProcess a WindowsApps exe (package
    // identity would be dropped — ChatGPT: "该进程没有程序包标识符").
    let req = SessionStartRequest {
        launch: crate::package_discovery::normalize_launch_target(&req.launch),
        ..req
    };
    let instance_id = requested_instance_id.unwrap_or_else(Uuid::new_v4);
    let profile_id = req.profile.as_ref().map(|p| p.id).unwrap_or_default();
    let host_mode = req.profile.is_none();

    if let Some(profile) = &req.profile {
        profile
            .validate()
            .map_err(|e| SessionError::InvalidProfile(e.to_string()))?;
        if profile.environment.len() > crate::ipc::RUNTIME_ENVIRONMENT_MAX {
            return Err(SessionError::InvalidProfile(format!(
                "environment has {} entries; Runtime supports at most {}",
                profile.environment.len(),
                crate::ipc::RUNTIME_ENVIRONMENT_MAX
            )));
        }
        for (key, value) in &profile.environment {
            if key.len() + 1 + value.len() >= crate::ipc::RUNTIME_ENVIRONMENT_ENTRY_MAX_BYTES {
                return Err(SessionError::InvalidProfile(format!(
                    "environment entry {key:?} is too large for Runtime IPC"
                )));
            }
        }
        let payload = crate::ipc::profile_to_message_with_flags(
            profile,
            &instance_id.to_string(),
            req.inherit_children,
            req.audit,
        )
        .encode_line();
        if payload.len() + 1 > crate::ipc::IPC_MAX_LINE_BYTES {
            return Err(SessionError::InvalidProfile(format!(
                "encoded Runtime Profile is {} bytes; IPC limit is {}",
                payload.len() + 1,
                crate::ipc::IPC_MAX_LINE_BYTES
            )));
        }
        // Strict WebRTC requires Network Guard (Runtime hooks_network). We inject
        // the Runtime on this path, so process-tree UDP deny is available.
        // Host-mode (no profile) never hits this check.
        let guard = envbox_core::NetworkGuardCapability::runtime_udp_enforced();
        guard
            .check_policy(profile.browser.webrtc)
            .map_err(SessionError::Unsupported)?;
    }

    let host_env: HashMap<String, String> = std::env::vars().collect();
    let is_packaged = matches!(req.launch, LaunchTarget::Packaged { .. });
    // Win32 receives a per-session pipe through its Environment Block.
    // Packaged roots derive a pipe from their PID after activation.
    let pipe_path =
        (!is_packaged).then(|| crate::ipc_server::session_pipe_name(&instance_id.to_string()));

    // Win32 starts the broker before its suspended process is created.
    let mut table = SessionTable::new();
    if let Some(profile) = &req.profile {
        table.set_instance_id(&instance_id.to_string());
        table.register_profile_flags(profile, req.inherit_children, req.audit);
    }
    let shared: crate::ipc_server::SharedTable = std::sync::Arc::new(std::sync::Mutex::new(table));
    let mut broker = if host_mode || is_packaged {
        None
    } else {
        match crate::ipc_server::HostBroker::start_on(
            shared.clone(),
            pipe_path.clone().expect("Win32 pipe path"),
        ) {
            Ok(b) => Some(b),
            Err(_) => None, // Win32 keeps its ENVBOX_* fallback.
        }
    };

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
        // Point Runtime IPC Bootstrap at this session's Host pipe (Win32 only).
        if let Some(pipe_path) = &pipe_path {
            env.insert("ENVBOX_IPC_PIPE".into(), pipe_path.clone());
        }
        env
    };

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

    // Open a pre-created named Job before activation so a missing/expired
    // Windows Terminal handoff fails closed without starting an untracked
    // target. Ordinary sessions create their private Job after activation.
    let mut job = match requested_job_name.as_deref() {
        Some(name) => InstanceJob::open_named(name)?,
        None => InstanceJob::create()?,
    };

    // Activation
    let runtime_dll = if host_mode {
        None
    } else {
        let source = crate::injection::resolve_runtime_dll()
            .map_err(|e| SessionError::Activate(ActivateError::Inject(e)))?;
        Some(
            crate::injection::stage_runtime_dll(&source, instance_id)
                .map_err(|e| SessionError::Activate(ActivateError::Inject(e)))?,
        )
    };

    let activation_req = ActivationRequest {
        arguments: req.arguments.clone(),
        working_directory: req.working_directory.clone(),
        environment: env.clone(),
        runtime_dll: runtime_dll.clone(),
        require_runtime: !host_mode,
        webrtc_policy: req.profile.as_ref().map(|p| p.browser.webrtc),
        browser_locale: req.profile.as_ref().map(|p| p.locale.locale_name.clone()),
        create_new_console,
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

    // No custom Environment Block reaches an AUMID target. The Runtime can
    // compute this PID-scoped path itself, avoiding another session's pipe.
    if !host_mode && is_packaged {
        let path = crate::ipc_server::packaged_pipe_name(activated.pid);
        broker = Some(
            crate::ipc_server::HostBroker::start_on(shared.clone(), path).map_err(|err| {
                SessionError::Unsupported(format!(
                    "IPC Broker bind failed for packaged root ({err})"
                ))
            })?,
        );
    }

    session.register_root(activated.pid);
    if let Some(ident) = activated.package_identity.clone() {
        session.package_identity = Some(ident);
    }
    session.state = SessionState::Activated;

    // Bind root PID BEFORE attach: DllMain GET_PROFILE runs during LoadLibrary.
    if req.profile.is_some() {
        shared
            .lock()
            .unwrap()
            .bind_pid(activated.pid, &profile_id.to_string());
    }

    // Process Tracker: Job for Win32, Package/PID for Packaged.
    let mut tracker = if session.package_identity.is_some() {
        crate::process_tracker::ProcessTracker::packaged(
            session
                .package_identity
                .clone()
                .expect("package identity checked"),
        )
    } else {
        crate::process_tracker::ProcessTracker::win32()
    };
    tracker.register_root(activated.pid);
    tracker.mark(SessionState::Activated);

    // Job (lifecycle only; best-effort for packaged). A named handoff Job is
    // required: silently proceeding without assignment would defeat GUI Stop.
    if let Err(err) = job.assign_pid(activated.pid) {
        if requested_job_name.is_some() {
            #[cfg(windows)]
            unsafe {
                use windows::Win32::System::Threading::TerminateProcess;
                let _ = TerminateProcess(activated.process.0, 1);
            }
            return Err(err.into());
        }
    }

    // Attach
    let attached = if host_mode {
        None
    } else {
        let dll = runtime_dll.clone().ok_or(SessionError::InjectionFailed)?;
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

    let profile_payload = req.profile.as_ref().map(|p| {
        crate::ipc::profile_to_message_with_flags(
            p,
            &instance_id.to_string(),
            req.inherit_children,
            req.audit,
        )
        .encode_line()
    });

    let mut process_ids = HashSet::new();
    process_ids.insert(activated.pid);
    let package_family_name = session
        .package_identity
        .as_ref()
        .map(|p| p.package_family_name.clone());
    let aumid = session.package_identity.as_ref().map(|p| p.aumid.clone());
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
        tracker,
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

/// Register a child on the Process Tracker (parent-aware).
pub fn register_child_tracked(
    tracker: &mut crate::process_tracker::ProcessTracker,
    pid: u32,
    parent: Option<u32>,
) {
    tracker.register_child(pid, parent);
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
    use envbox_core::{DnsMode, DnsProfile, LocaleProfile, RegistryProfile, TimezoneProfile};

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
            browser: Default::default(),
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

    #[test]
    fn strict_webrtc_requires_network_guard_startup_fail() {
        // Direct check: without guard capability, Strict is refused.
        let refused = envbox_core::NetworkGuardCapability::browser_policy_only()
            .check_policy(envbox_core::WebRtcPolicy::Strict);
        assert!(refused.is_err());

        // start_session path uses runtime_udp_enforced (Runtime hooks_network),
        // so Strict is accepted at the policy gate (launch may still need DLL).
        let mut p = profile();
        p.browser.webrtc = envbox_core::WebRtcPolicy::Strict;
        envbox_core::NetworkGuardCapability::runtime_udp_enforced()
            .check_policy(p.browser.webrtc)
            .expect("strict ok when network guard present");
    }

    #[test]
    fn proxy_only_still_starts_policy_check_ok() {
        // Policy check itself must accept ProxyOnly (actual launch may still
        // need a runtime DLL — that is a separate gate).
        let mut p = profile();
        p.browser.webrtc = envbox_core::WebRtcPolicy::ProxyOnly;
        envbox_core::NetworkGuardCapability::browser_policy_only()
            .check_policy(p.browser.webrtc)
            .expect("proxy_only must not require network guard");
    }

    #[test]
    fn windows_apps_executable_is_normalized_to_packaged() {
        let req = SessionStartRequest {
            application_id: Uuid::nil(),
            launch: LaunchTarget::Executable {
                path: r"C:\Program Files\WindowsApps\OpenAI.Codex_26.924.1866.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
                    .into(),
            },
            arguments: vec![],
            working_directory: None,
            profile: None,
            inherit_children: true,
            audit: false,
        };
        // start_session normalizes before any create; we only assert the mapping
        // here (full start needs a live package).
        let launch = crate::package_discovery::normalize_launch_target(&req.launch);
        match launch {
            LaunchTarget::Packaged { aumid, .. } => {
                assert_eq!(aumid, "OpenAI.Codex_2p2nqsd0c76g0!App")
            }
            other => panic!("expected Packaged, got {other:?}"),
        }
    }

    /// Machine smoke: Full Trust packaged ChatGPT via AUMID (issue 36 / GUI run).
    #[test]
    #[ignore = "launches real ChatGPT; run explicitly"]
    fn chatgpt_packaged_aumid_starts() {
        let launch = crate::package_discovery::launch_target_from_user_path(
            r"shell:AppsFolder\OpenAi.Codex_2p2nqsd0c76g0!App",
        );
        assert!(
            matches!(launch, LaunchTarget::Packaged { .. }),
            "{launch:?}"
        );
        let req = SessionStartRequest {
            application_id: Uuid::nil(),
            launch,
            arguments: vec![],
            working_directory: None,
            profile: Some(profile()),
            inherit_children: true,
            audit: false,
        };
        let mut handle = start_session(req).expect("packaged session must start");
        assert!(handle.instance.root_pid > 0);
        assert!(handle.instance.aumid.is_some());
        let _ = handle.job.as_mut().map(|j| j.terminate());
    }
}
