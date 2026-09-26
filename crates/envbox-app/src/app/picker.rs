//! Installed application picker and icon loading.

use std::path::PathBuf;
use uuid::Uuid;
use iced::Task;
use envbox_core::LaunchTarget;
use crate::message::{LaunchKind, Message, StatusKind};
use super::{AppDraft, EnvBoxApp};

impl EnvBoxApp {
    /// Icon cache directory beside config store.
    pub fn icon_dir(&self) -> PathBuf {
        self.store.root().join("icons")
    }

    /// Scan into the picker off-thread (Kite-inspired). List first, icons later.
    pub(super) fn scan_apps_for_picker(&mut self) -> Task<Message> {
        if let Some(p) = self.app_picker.as_mut() {
            p.loading = true;
            p.scanned = false;
            p.items.clear();
            p.selected = None;
        }
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let items = crate::discover::scan_installed_apps();
                    let _ = tx.send(items);
                });
                rx.await.unwrap_or_default()
            },
            Message::AppPickerLoaded,
        )
    }

    /// Parallel icon extraction for picker rows (after the list is visible).
    pub(super) fn extract_picker_icons(&mut self) -> Task<Message> {
        let icon_dir = self.icon_dir();
        let Some(p) = self.app_picker.as_ref() else {
            return Task::none();
        };
        if p.items.is_empty() {
            return Task::none();
        }
        let pending: Vec<(String, String, String)> = p
            .items
            .iter()
            .filter(|a| a.icon_png.is_none())
            .take(200)
            .map(|a| (a.icon_key(), a.icon_src.clone(), a.path.clone()))
            .collect();
        if pending.is_empty() {
            return Task::none();
        }
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    use std::sync::{Arc, Mutex};
                    let results = Arc::new(Mutex::new(Vec::<(String, Option<std::path::PathBuf>)>::new()));
                    if pending.is_empty() {
                        let _ = tx.send(Vec::new());
                        return;
                    }
                    let chunk = pending.len().div_ceil(6).max(1);
                    std::thread::scope(|s| {
                        for part in pending.chunks(chunk) {
                            let part = part.to_vec();
                            let icon_dir = icon_dir.clone();
                            let results = Arc::clone(&results);
                            s.spawn(move || {
                                for (key, src, _path) in &part {
                                    let png =
                                        crate::app_icon::cache_app_icon(&icon_dir, key, src);
                                    if let Ok(mut g) = results.lock() {
                                        g.push((key.clone(), png));
                                    }
                                }
                            });
                        }
                    });
                    let out: Vec<(String, std::path::PathBuf)> = results
                        .lock()
                        .map(|g| g.clone())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|(k, p)| p.map(|p| (k, p)))
                        .collect();
                    let _ = tx.send(out);
                });
                rx.await.unwrap_or_default()
            },
            Message::AppPickerIcons,
        )
    }

    /// Background-extract icons for configured applications (cards + detail).
    pub fn refresh_app_icons(&mut self) -> Task<Message> {
        let icon_dir = self.icon_dir();
        let targets: Vec<(Uuid, String)> = self
            .applications
            .iter()
            .map(|a| {
                let src = match &a.launch {
                    LaunchTarget::Command { command } => command.clone(),
                    LaunchTarget::Executable { path } => path.display().to_string(),
                    LaunchTarget::Packaged { aumid, .. } => aumid.clone(),
                };
                (a.id, src)
            })
            .collect();
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let mut out = Vec::new();
                    for (id, src) in targets {
                        if let Some(png) = crate::app_icon::cache_app_icon(&icon_dir, &id.to_string(), &src)
                        {
                            out.push((id, png));
                        }
                    }
                    let _ = tx.send(out);
                });
                rx.await.unwrap_or_default()
            },
            Message::AppIconsLoaded,
        )
    }

    pub fn filtered_picker_items(&self) -> Vec<(usize, &crate::discover::DiscoveredApp)> {
        let Some(p) = self.app_picker.as_ref() else {
            return Vec::new();
        };
        let q = p.query.trim().to_ascii_lowercase();
        p.items
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                q.is_empty()
                    || a.name.to_ascii_lowercase().contains(&q)
                    || a.path.to_ascii_lowercase().contains(&q)
                    || a.args.to_ascii_lowercase().contains(&q)
                    || a.source.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    /// Fill `app_draft` from the selected discovered entry.
    pub(super) fn apply_picker_selection(&mut self) -> Task<Message> {
        let Some(idx) = self.app_picker.as_ref().and_then(|p| p.selected) else {
            self.set_status(StatusKind::Error, "请先在列表中选择一个应用");
            return Task::none();
        };
        let Some(item) = self
            .app_picker
            .as_ref()
            .and_then(|p| p.items.get(idx).cloned())
        else {
            self.set_status(StatusKind::Error, "选择的应用无效");
            return Task::none();
        };

        let profile_id = self.profiles.first().map(|p| p.id).unwrap_or_default();
        let cap = item.capability;
        self.app_draft = AppDraft {
            id: None,
            name: item.name.clone(),
            kind: if matches!(
                cap.packaging,
                crate::package::Packaging::PackagedWin32
                    | crate::package::Packaging::AppContainer
                    | crate::package::Packaging::PackagedUnknown
            ) {
                LaunchKind::Packaged
            } else {
                LaunchKind::Executable
            },
            path: item.path.clone(),
            args: item.args.clone(),
            work_dir: item.work_dir.clone(),
            profile_id,
            inherit: true,
            audit: false,
            icon_src: item.icon_src.clone(),
        };
        self.app_saved_draft = self.app_draft.clone();
        self.app_edit_mode = true;
        self.app_advanced = false;
        self.app_picker = None;
        match cap.injection {
            crate::package::InjectionSupport::Supported => {
                self.set_status(
                    StatusKind::Success,
                    format!("已带入「{}」，请确认默认环境后保存", item.name),
                );
            }
            crate::package::InjectionSupport::Delayed => {
                self.set_status(
                    StatusKind::Info,
                    format!("已带入「{}」。{}", item.name, cap.user_explanation()),
                );
            }
            crate::package::InjectionSupport::Unsupported => {
                self.set_status(
                    StatusKind::Info,
                    format!("已带入「{}」。{}", item.name, cap.user_explanation()),
                );
            }
        }
        Task::none()
    }

}
