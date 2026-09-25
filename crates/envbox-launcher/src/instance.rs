//! RuntimeInstance lifecycle for GUI/CLI (ticket 10).
//! Job Object tracks the Process Tree Instance; not a security boundary.
//! Packaged / AUMID roots go through `start_session` (ActivationBackend).

use crate::job::{JobError, JobStats};
use crate::launcher::LaunchError;
use crate::package_discovery::normalize_launch_target;
use crate::session::{start_session, SessionError, SessionHandle, SessionStartRequest};
use envbox_core::{Application, EnvironmentProfile, InstanceStatus, RuntimeInstance};
use std::collections::HashMap;
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
    pub session: SessionHandle,
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
        let request = build_session_request(app, target);
        let session = start_session(request)?;
        let id = session.instance.id;
        let meta = session.instance.clone();
        self.instances.insert(id, InstanceHandle { meta, session });
        Ok(id)
    }

    /// Stop the Process Tree Instance by closing the Job
    /// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). On failure status becomes Failed
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
        match handle.session.job.as_mut().map(|j| j.close()) {
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
        let Some(job) = handle.session.job.as_ref() else {
            return Ok(handle.meta.status);
        };
        let stats = job.stats()?;
        handle.meta.status = status_from_stats(&stats, handle.meta.status);
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
        let Some(job) = handle.session.job.as_ref() else {
            return Ok(0);
        };
        Ok(children_from_active_processes(job.stats()?.active_processes))
    }
}

/// Job `active_processes` includes Root Process; story 19 reports children only.
fn children_from_active_processes(active: u32) -> u32 {
    active.saturating_sub(1)
}

/// Build SessionStartRequest. Host → `profile: None` (no injection).
/// AUMID / shell:AppsFolder paths normalize to `LaunchTarget::Packaged`.
fn build_session_request(app: &Application, target: RunTarget) -> SessionStartRequest {
    let (profile, inherit_children) = match &target {
        RunTarget::Profile(p) => (Some(p.clone()), app.inherit_children),
        RunTarget::Host => (None, app.inherit_children),
    };
    SessionStartRequest {
        application_id: app.id,
        launch: normalize_launch_target(&app.launch),
        arguments: app.arguments.clone(),
        working_directory: app.working_directory.clone(),
        profile,
        inherit_children,
        audit: app.audit,
    }
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
        let req = build_session_request(&app, RunTarget::Host);
        assert!(req.profile.is_none(), "Host must not carry a Profile");
        assert_eq!(req.arguments, app.arguments);
    }

    #[test]
    fn profile_target_carries_profile_on_session_request() {
        let profile = sample_profile();
        let profile_id = profile.id;
        let app = sample_app(profile_id);
        let req = build_session_request(&app, RunTarget::Profile(profile));
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
            working_directory: None,
            arguments: vec![],
            default_profile_id: Uuid::nil(),
            inherit_children: true,
            audit: false,
        };
        let req = build_session_request(&app, RunTarget::Host);
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
}
