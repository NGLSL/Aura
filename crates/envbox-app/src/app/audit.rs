//! Audit loading and software labels used by the audit view and filter.

use super::EnvBoxApp;
use crate::message::{Message, StatusKind};
use envbox_core::AuditEvent;
use iced::Task;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct AuditSnapshot {
    pub events: Vec<AuditEvent>,
    pub total: usize,
}

impl AuditSnapshot {
    fn load(dir: PathBuf) -> Self {
        const AUDIT_LIMIT: usize = 500;
        let mut snapshot = Self::default();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return snapshot;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
            .collect();
        files.sort();
        let mut latest = BTreeMap::new();
        for path in files {
            let Ok(file) = std::fs::File::open(path) else {
                continue;
            };
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else {
                    continue;
                };
                if let Ok(ev) = AuditEvent::parse_json_line(&line) {
                    snapshot.total += 1;
                    // UTC timestamps use the same ISO format. The sequence makes
                    // equal timestamps deterministic and keeps later records first.
                    let key = (ev.ts_utc.clone(), snapshot.total);
                    if latest.len() < AUDIT_LIMIT
                        || latest
                            .first_key_value()
                            .is_some_and(|(oldest, _)| key > *oldest)
                    {
                        latest.insert(key, ev);
                        if latest.len() > AUDIT_LIMIT {
                            latest.pop_first();
                        }
                    }
                }
            }
        }
        snapshot.events = latest.into_values().rev().collect();
        snapshot
    }
}

impl EnvBoxApp {
    pub(super) fn load_audit(&mut self) -> Task<Message> {
        // Coalesce repeated requests instead of starting competing full scans.
        if self.audit_loading {
            self.audit_reload_requested = true;
            return Task::none();
        }
        self.audit_loading = true;
        self.audit_generation = self.audit_generation.wrapping_add(1);
        let generation = self.audit_generation;
        let directory = self.store.audit_dir();
        Task::perform(
            async move {
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                let worker = std::thread::Builder::new()
                    .name("aura-audit-load".into())
                    .spawn(move || {
                        let _ = sender.send(AuditSnapshot::load(directory));
                    });
                worker.map_err(|error| format!("无法启动审计加载任务：{error}"))?;
                receiver
                    .await
                    .map_err(|_| "审计加载任务意外结束".to_string())
            },
            move |result| Message::AuditLoaded(generation, result),
        )
    }

    pub(super) fn finish_audit_load(
        &mut self,
        generation: u64,
        result: Result<AuditSnapshot, String>,
    ) -> Task<Message> {
        if generation != self.audit_generation || !self.audit_loading {
            return Task::none();
        }
        self.audit_loading = false;
        match result {
            Ok(snapshot) => {
                self.audit_events = snapshot.events;
                self.audit_total = snapshot.total;
            }
            Err(error) => self.set_status(StatusKind::Error, error),
        }
        if std::mem::take(&mut self.audit_reload_requested) {
            return self.load_audit();
        }
        Task::none()
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
