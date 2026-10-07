//! Audit loading and software labels used by the audit view and filter.

use super::EnvBoxApp;
use envbox_core::AuditEvent;
use std::path::PathBuf;

impl EnvBoxApp {
    pub fn load_audit(&mut self) {
        const AUDIT_LIMIT: usize = 500;
        let dir = self.store.audit_dir();
        self.audit_events.clear();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
            .collect();
        files.sort();
        for path in files.iter().rev() {
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            for line in content.lines() {
                if let Ok(ev) = AuditEvent::parse_json_line(line) {
                    self.audit_events.push(ev);
                }
            }
            if self.audit_events.len() >= AUDIT_LIMIT {
                break;
            }
        }
        // Newest first in the table; keep the most recent AUDIT_LIMIT.
        self.audit_events.reverse();
        self.audit_events.truncate(AUDIT_LIMIT);
    }

    /// Display label for an audit row: configured app name if image matches, else image, else PID.
    pub fn audit_software_label(&self, ev: &AuditEvent) -> String {
        if let Some(img) = ev
            .image
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            for a in &self.applications {
                let matches = match &a.launch {
                    envbox_core::LaunchTarget::Executable { path } => path
                        .file_name()
                        .map(|n| n.to_string_lossy().eq_ignore_ascii_case(img))
                        .unwrap_or(false),
                    envbox_core::LaunchTarget::Command { command } => PathBuf::from(command)
                        .file_name()
                        .map(|n| n.to_string_lossy().eq_ignore_ascii_case(img))
                        .unwrap_or(false),
                    envbox_core::LaunchTarget::Packaged { aumid, .. } => aumid
                        .rsplit(['!', '\\', '/'])
                        .next()
                        .map(|n| {
                            let n = n.trim_end_matches(".exe");
                            img.trim_end_matches(".exe").eq_ignore_ascii_case(n)
                        })
                        .unwrap_or(false),
                };
                if matches {
                    return a.name.clone();
                }
            }
            return img.to_string();
        }
        format!("#{}", ev.pid)
    }

    /// Distinct software labels present in loaded audit events (sorted).
    pub fn audit_software_options(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .audit_events
            .iter()
            .map(|ev| self.audit_software_label(ev))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Audit rows after per-software filter (newest first).
    pub fn filtered_audit_events(&self) -> Vec<&AuditEvent> {
        let q = self.audit_filter.trim();
        self.audit_events
            .iter()
            .filter(|ev| {
                if q.is_empty() || q == "全部软件" {
                    return true;
                }
                self.audit_software_label(ev).eq_ignore_ascii_case(q)
            })
            .collect()
    }
}
