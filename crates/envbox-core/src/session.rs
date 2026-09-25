//! EnvironmentSession control-plane contracts (packaged-v1 spec).
//! Launch and attach are separate; capabilities decide strategy.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use uuid::Uuid;

use crate::LaunchTarget;

/// Package identity for MSIX / WindowsApps targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub aumid: String,
    pub package_full_name: String,
    pub package_family_name: String,
}

/// How a process was started — determines attach timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachStrategy {
    /// CreateProcess suspended → inject → resume (Win32).
    PreExecution,
    /// Activate first, then inject (packaged root race window).
    PostActivation,
    /// Reserved: IPackageDebugSettings / experimental.
    PackageDebug,
}

/// What isolation this session actually delivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IsolationGuarantee {
    FullPreExecution,
    PostActivation,
    Partial,
}

/// Capability flags — selection is data-driven, not path-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TargetCapabilities {
    pub can_suspend: bool,
    pub can_inject_runtime: bool,
    pub can_create_environment_block: bool,
    pub can_assign_job: bool,
    pub can_track_children: bool,
}

impl TargetCapabilities {
    /// Classic Win32 desktop process: full pre-execution path.
    pub fn win32() -> Self {
        Self {
            can_suspend: true,
            can_inject_runtime: true,
            can_create_environment_block: true,
            can_assign_job: true,
            can_track_children: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityLevel {
    Unknown,
    Low,
    Medium,
    High,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MitigationPolicy {
    Unknown,
    Off,
    /// Blocks third-party runtime images (MicrosoftSignedOnly / StoreSignedOnly / etc).
    Blocking,
    Allow,
}

/// Process-level injection capability (after activation / before attach).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectionCapability {
    pub is_packaged: bool,
    pub is_app_container: bool,
    pub integrity_level: IntegrityLevel,
    pub signature_policy: MitigationPolicy,
    pub dynamic_code_policy: MitigationPolicy,
    pub image_load_policy: MitigationPolicy,
    pub supported: bool,
    pub reason: Option<String>,
}

/// Target as activated (process exists; may be suspended).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivatedTarget {
    pub pid: u32,
    pub package_identity: Option<PackageIdentity>,
    pub suspended: bool,
    pub capabilities: TargetCapabilities,
    pub injection: InjectionCapability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Created,
    Activated,
    Attached,
    Running,
    Stopping,
    Exited,
    Failed,
}

/// Control-plane aggregate for one Run (see packaged-v1 spec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSession {
    pub id: Uuid,
    pub application_id: Uuid,
    pub target: LaunchTarget,
    pub profile_id: Uuid,
    pub root_processes: HashSet<u32>,
    pub processes: HashSet<u32>,
    pub package_identity: Option<PackageIdentity>,
    pub isolation: IsolationGuarantee,
    pub attach_strategy: AttachStrategy,
    pub state: SessionState,
}

impl EnvironmentSession {
    pub fn new(
        application_id: Uuid,
        target: LaunchTarget,
        profile_id: Uuid,
        isolation: IsolationGuarantee,
        attach_strategy: AttachStrategy,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            application_id,
            target,
            profile_id,
            root_processes: HashSet::new(),
            processes: HashSet::new(),
            package_identity: None,
            isolation,
            attach_strategy,
            state: SessionState::Created,
        }
    }

    pub fn register_root(&mut self, pid: u32) {
        self.root_processes.insert(pid);
        self.processes.insert(pid);
    }

    pub fn register_child(&mut self, pid: u32) {
        self.processes.insert(pid);
    }

    /// Session membership: ancestry is tracked separately; packaged sessions
    /// may also own processes via package family + creation window.
    pub fn belongs_to_session(
        &self,
        pid: u32,
        is_descendant: bool,
        package_family: Option<&str>,
        created_in_activation_window: bool,
    ) -> bool {
        if self.processes.contains(&pid) || is_descendant {
            return true;
        }
        match (&self.package_identity, package_family) {
            (Some(ident), Some(fam)) => {
                ident.package_family_name.eq_ignore_ascii_case(fam)
                    && created_in_activation_window
            }
            _ => false,
        }
    }
}

/// Choose attach strategy from capabilities (no path/type branches).
pub fn select_attach_strategy(caps: &TargetCapabilities) -> Option<AttachStrategy> {
    if caps.can_suspend && caps.can_inject_runtime {
        return Some(AttachStrategy::PreExecution);
    }
    if caps.can_inject_runtime {
        return Some(AttachStrategy::PostActivation);
    }
    None
}

pub fn isolation_for_strategy(strategy: AttachStrategy) -> IsolationGuarantee {
    match strategy {
        AttachStrategy::PreExecution => IsolationGuarantee::FullPreExecution,
        AttachStrategy::PostActivation => IsolationGuarantee::PostActivation,
        // Reserved path is not a stronger guarantee than PreExecution.
        AttachStrategy::PackageDebug => IsolationGuarantee::Partial,
    }
}

/// Pure policy: can this process receive envbox-runtime?
/// (Prototype decision from packaged-v1 — no bypass.)
pub fn evaluate_injection_support(
    is_app_container: bool,
    signature_policy: MitigationPolicy,
    dynamic_code_policy: MitigationPolicy,
    image_load_policy: MitigationPolicy,
    integrity_ok: bool,
    can_open_process: bool,
) -> (bool, Option<String>) {
    if !can_open_process {
        return (false, Some("cannot open/query target process".into()));
    }
    if is_app_container {
        return (
            false,
            Some("AppContainer: Runtime injection unsupported".into()),
        );
    }
    if !integrity_ok {
        return (
            false,
            Some("integrity/elevation: cannot inject into target".into()),
        );
    }
    for (name, p) in [
        ("signature", signature_policy),
        ("dynamic_code", dynamic_code_policy),
        ("image_load", image_load_policy),
    ] {
        // Fail closed on Unknown as well as Blocking (spec: No bypass).
        if matches!(p, MitigationPolicy::Blocking | MitigationPolicy::Unknown) {
            return (
                false,
                Some(format!("{name} mitigation unknown or blocks third-party runtime image load")),
            );
        }
    }
    (true, None)
}

/// Derive TargetCapabilities from packaging + injection probe result.
pub fn capabilities_for_target(
    is_packaged: bool,
    injection_supported: bool,
    attach: Option<AttachStrategy>,
) -> TargetCapabilities {
    let _ = is_packaged;
    match attach {
        Some(AttachStrategy::PreExecution) => TargetCapabilities::win32(),
        Some(AttachStrategy::PostActivation) | Some(AttachStrategy::PackageDebug) => {
            TargetCapabilities {
                can_suspend: false,
                can_inject_runtime: injection_supported,
                can_create_environment_block: false,
                can_assign_job: true,
                can_track_children: true,
            }
        }
        None => TargetCapabilities::default(),
    }
}

/// Classic Win32 launch target helpers (serde-compatible names).
pub fn is_packaged_target(target: &LaunchTarget) -> bool {
    matches!(target, LaunchTarget::Packaged { .. })
}

pub fn win32_exe(path: PathBuf) -> LaunchTarget {
    LaunchTarget::Executable { path }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_pre_execution_when_can_suspend() {
        let caps = TargetCapabilities::win32();
        assert_eq!(
            select_attach_strategy(&caps),
            Some(AttachStrategy::PreExecution)
        );
    }

    #[test]
    fn select_post_activation_when_cannot_suspend() {
        let caps = TargetCapabilities {
            can_suspend: false,
            can_inject_runtime: true,
            can_create_environment_block: false,
            can_assign_job: true,
            can_track_children: true,
        };
        assert_eq!(
            select_attach_strategy(&caps),
            Some(AttachStrategy::PostActivation)
        );
    }

    #[test]
    fn select_none_when_cannot_inject() {
        let caps = TargetCapabilities::default();
        assert_eq!(select_attach_strategy(&caps), None);
    }

    #[test]
    fn app_container_is_unsupported() {
        let (ok, reason) = evaluate_injection_support(
            true,
            MitigationPolicy::Allow,
            MitigationPolicy::Allow,
            MitigationPolicy::Allow,
            true,
            true,
        );
        assert!(!ok);
        assert!(reason.unwrap().contains("AppContainer"));
    }

    #[test]
    fn blocking_signature_is_unsupported() {
        let (ok, reason) = evaluate_injection_support(
            false,
            MitigationPolicy::Blocking,
            MitigationPolicy::Allow,
            MitigationPolicy::Allow,
            true,
            true,
        );
        assert!(!ok);
        assert!(reason.unwrap().contains("signature"));
    }

    #[test]
    fn medium_clean_is_supported() {
        let (ok, reason) = evaluate_injection_support(
            false,
            MitigationPolicy::Allow,
            MitigationPolicy::Allow,
            MitigationPolicy::Allow,
            true,
            true,
        );
        assert!(ok);
        assert!(reason.is_none());
    }

    #[test]
    fn unknown_mitigation_is_fail_closed() {
        let (ok, reason) = evaluate_injection_support(
            false,
            MitigationPolicy::Unknown,
            MitigationPolicy::Allow,
            MitigationPolicy::Allow,
            true,
            true,
        );
        assert!(!ok);
        assert!(reason.unwrap().contains("unknown"));
    }

    #[test]
    fn session_package_membership_needs_window() {
        let mut s = EnvironmentSession::new(
            Uuid::nil(),
            LaunchTarget::Packaged {
                aumid: "Foo_bar!App".into(),
                package_full_name: "Foo_bar_1.0.0.0_x64__abc".into(),
                package_family_name: "Foo_bar_abc".into(),
            },
            Uuid::nil(),
            IsolationGuarantee::PostActivation,
            AttachStrategy::PostActivation,
        );
        s.package_identity = Some(PackageIdentity {
            aumid: "Foo_bar!App".into(),
            package_full_name: "Foo_bar_1.0.0.0_x64__abc".into(),
            package_family_name: "Foo_bar_abc".into(),
        });
        assert!(s.belongs_to_session(1, true, None, false));
        assert!(s.belongs_to_session(2, false, Some("Foo_bar_abc"), true));
        assert!(!s.belongs_to_session(3, false, Some("Foo_bar_abc"), false));
        assert!(!s.belongs_to_session(4, false, Some("Other_pkg"), true));
    }

    #[test]
    fn isolation_for_post_activation() {
        assert_eq!(
            isolation_for_strategy(AttachStrategy::PostActivation),
            IsolationGuarantee::PostActivation
        );
        assert_eq!(
            isolation_for_strategy(AttachStrategy::PreExecution),
            IsolationGuarantee::FullPreExecution
        );
    }
}
