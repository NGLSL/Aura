//! Process Tracker (V0.3 ticket 47).
//!
//! Unified lifecycle for an EnvironmentSession:
//! - Win32: Job Object (best-effort kill/stop/stats)
//! - Packaged: PID + Package Identity set (Job may be partial)
//!
//! Job Object is a lifecycle tool, not a security boundary.

use envbox_core::{PackageIdentity, SessionState};
use std::collections::HashSet;

/// How a tracked process set is supervised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackMode {
    /// Classic Win32 Process Tree Instance via Job Object.
    Job,
    /// Packaged / post-activation: PID + package family membership only.
    PackagePid,
}

/// One EnvironmentSession's process set + supervision mode.
#[derive(Debug, Clone)]
pub struct ProcessTracker {
    pub mode: TrackMode,
    pub roots: HashSet<u32>,
    pub processes: HashSet<u32>,
    pub package: Option<PackageIdentity>,
    /// Activation window start (for package-family membership).
    pub activated_at: std::time::SystemTime,
    pub state: SessionState,
}

impl ProcessTracker {
    pub fn new(mode: TrackMode, package: Option<PackageIdentity>) -> Self {
        Self {
            mode,
            roots: HashSet::new(),
            processes: HashSet::new(),
            package,
            activated_at: std::time::SystemTime::now(),
            state: SessionState::Created,
        }
    }

    pub fn win32() -> Self {
        Self::new(TrackMode::Job, None)
    }

    pub fn packaged(package: PackageIdentity) -> Self {
        Self::new(TrackMode::PackagePid, Some(package))
    }

    pub fn register_root(&mut self, pid: u32) {
        self.roots.insert(pid);
        self.processes.insert(pid);
    }

    pub fn register_child(&mut self, pid: u32, parent: Option<u32>) {
        // Only parent-tracked children join (ancestry). Orphans need a
        // separate package-window check via `belongs`.
        if let Some(parent) = parent {
            if self.processes.contains(&parent) || self.roots.contains(&parent) {
                self.processes.insert(pid);
            }
            return;
        }
        // No parent info: do not silently claim the pid.
    }

    pub fn on_exit(&mut self, pid: u32) {
        self.processes.remove(&pid);
        self.roots.remove(&pid);
    }

    /// Membership: tracked set, descendant of a root, or package window match.
    pub fn belongs(
        &self,
        pid: u32,
        is_descendant: bool,
        package_family: Option<&str>,
        created_in_activation_window: bool,
    ) -> bool {
        if self.processes.contains(&pid) || is_descendant {
            return true;
        }
        match (&self.package, package_family) {
            (Some(ident), Some(fam)) => {
                ident.package_family_name.eq_ignore_ascii_case(fam) && created_in_activation_window
            }
            _ => false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    pub fn mark(&mut self, state: SessionState) {
        self.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> PackageIdentity {
        PackageIdentity {
            aumid: "Foo!App".into(),
            package_full_name: "Foo_1".into(),
            package_family_name: "Foo_abc".into(),
        }
    }

    #[test]
    fn job_tracker_follows_parent_child() {
        let mut t = ProcessTracker::win32();
        t.register_root(1);
        t.register_child(2, Some(1));
        t.register_child(3, Some(2));
        assert!(t.belongs(3, false, None, false));
        t.on_exit(3);
        assert!(!t.processes.contains(&3));
    }

    #[test]
    fn package_tracker_needs_window_for_orphans() {
        let mut t = ProcessTracker::packaged(pkg());
        t.register_root(10);
        assert!(t.belongs(11, true, None, false));
        assert!(t.belongs(12, false, Some("Foo_abc"), true));
        assert!(!t.belongs(13, false, Some("Foo_abc"), false));
        assert!(!t.belongs(14, false, Some("other"), true));
    }

    #[test]
    fn track_mode_matches_activation() {
        assert_eq!(ProcessTracker::win32().mode, TrackMode::Job);
        assert_eq!(ProcessTracker::packaged(pkg()).mode, TrackMode::PackagePid);
    }
}
