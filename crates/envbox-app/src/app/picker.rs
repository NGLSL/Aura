//! Installed application picker and icon loading.

use envbox_core::ConsoleHost;
use iced::Task;
use std::path::PathBuf;

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
            p.icon_requests.clear();
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
        let limit = if p.query.trim().is_empty() { 200 } else { 12 };
        let candidates: Vec<(String, String)> = self
            .filtered_picker_items()
            .into_iter()
            .map(|(_, a)| a)
            .filter(|a| a.icon_png.is_none())
            .take(limit)
            .map(|a| (a.icon_key(), a.icon_src.clone()))
            .collect();
        let Some(p) = self.app_picker.as_mut() else {
            return Task::none();
        };
        let pending: Vec<_> = candidates
            .into_iter()
            .filter(|(key, _)| p.icon_requests.insert(key.clone()))
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
                                for (key, src) in &part {
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
        let targets = self.applications.clone();
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let mut out = Vec::new();
                    for app in targets {
                        if let Some(png) = crate::app_icon::cache_application_icon(&icon_dir, &app)
                        {
                            out.push((app.id, png));
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
        let mut matches: Vec<_> = p.items
            .iter()
            .enumerate()
            .filter(|(_, a)| a.matches_picker_query(&q))
            .collect();
        if !q.is_empty() {
            matches.sort_by_key(|(_, a)| {
                let name = a.name.to_ascii_lowercase();
                if name == q {
                    0
                } else if name.starts_with(&q) {
                    1
                } else if name.contains(&q) {
                    2
                } else {
                    3
                }
            });
        }
        matches
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

        if item.is_windows_terminal_package() {
            if let Some(picker) = self.app_picker.as_mut() {
                picker.terminal_shells = Some(crate::discover_commands::installed_terminal_shells());
            }
            return Task::none();
        }

        let profile_id = self.profiles.first().map(|p| p.id).unwrap_or_default();
        let cap = item.capability;
        let draft = AppDraft {
            id: None,
            name: item.name.clone(),
            kind: if item.source == "commands" {
                // .cmd/.bat wrappers need the Command resolver's ComSpec path.
                LaunchKind::Command
            } else if matches!(
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
            chrome_independent_data: false,
            console_host: ConsoleHost::Direct,
            args: item.args.clone(),
            work_dir: item.work_dir.clone(),
            profile_id,
            inherit: true,
            audit: false,
            icon_src: item.icon_src.clone(),
        };
        self.open_picked_application(draft);
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

    pub(super) fn apply_terminal_shell_selection(&mut self, index: Option<usize>) -> Task<Message> {
        let Some(shells) = self
            .app_picker
            .as_ref()
            .and_then(|picker| picker.terminal_shells.as_ref())
        else {
            return Task::none();
        };
        let profile_id = self.profiles.first().map(|p| p.id).unwrap_or_default();
        let mut draft = AppDraft::blank(profile_id);
        draft.console_host = ConsoleHost::WindowsTerminal;
        if let Some(index) = index {
            let Some(shell) = shells.get(index) else {
                return Task::none();
            };
            draft.name = shell.name.to_string();
            draft.path = shell.command.clone();
            draft.icon_src = shell.command.clone();
        }
        let status = if index.is_some() {
            "已填入 Shell 和终端宿主，请确认默认环境后保存"
        } else {
            "请填写 Shell 名称和启动命令"
        };
        self.open_picked_application(draft);
        self.set_status(StatusKind::Info, status);
        Task::none()
    }

    fn open_picked_application(&mut self, draft: AppDraft) {
        self.app_saved_draft = draft.clone();
        self.app_draft = draft;
        self.app_edit_mode = true;
        self.app_advanced = false;
        self.app_picker = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppPickerState;
    use crate::discover::DiscoveredApp;
    use crate::discover_commands::TerminalShell;
    use envbox_core::{DnsMode, DnsProfile, EnvironmentProfile, LaunchTarget, LocaleProfile, TimezoneProfile};
    use envbox_storage::{ConfigStore, ProfileDocument};
    use uuid::Uuid;

    #[test]
    fn store_terminal_selection_saves_a_command_with_the_terminal_host() {
        let root = std::env::temp_dir().join(format!("aura-terminal-picker-{}", Uuid::new_v4()));
        let store = ConfigStore::new(&root);
        let profile_id = Uuid::new_v4();
        store
            .save_profiles(&ProfileDocument {
                profiles: vec![EnvironmentProfile {
                    id: profile_id,
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
                    dns: DnsProfile::from_servers(DnsMode::Host, vec![]),
                    identity: Default::default(),
                    environment: Default::default(),
                    registry: Default::default(),
                    browser: Default::default(),
                }],
            })
            .unwrap();
        let (mut app, _) = EnvBoxApp::new_with_store(store.clone());
        let terminal = r"shell:AppsFolder\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App";
        app.app_picker = Some(AppPickerState {
            items: vec![DiscoveredApp {
                name: "终端".into(),
                path: terminal.into(),
                args: String::new(),
                work_dir: String::new(),
                icon_src: terminal.into(),
                source: "apps-folder",
                capability: crate::package::classify_target(terminal, ""),
                icon_png: None,
            }],
            query: "terminal".into(),
            selected: Some(0),
            ..AppPickerState::default()
        });

        let _ = app.update(Message::AppPickerUseSelected);
        assert!(app.app_picker.as_ref().unwrap().terminal_shells.is_some());
        assert!(app.app_draft.path.is_empty());
        let _ = app.update(Message::AppPickerBack);
        let picker = app.app_picker.as_ref().unwrap();
        assert!(picker.terminal_shells.is_none());
        assert_eq!(picker.query, "terminal");
        assert_eq!(picker.selected, Some(0));

        let _ = app.update(Message::AppPickerUseSelected);
        let command = r"D:\Program Files\PowerShell\7\pwsh.exe";
        app.app_picker.as_mut().unwrap().terminal_shells = Some(vec![TerminalShell {
            name: "PowerShell 7",
            command: command.into(),
        }]);
        let _ = app.update(Message::AppPickerUseTerminalShell(Some(0)));
        assert!(app.app_picker.is_none());
        assert!(app.app_edit_mode);
        let _ = app.update(Message::AppSave);

        let saved = store.load_applications().unwrap();
        let [application] = saved.applications.as_slice() else {
            panic!("expected one saved PowerShell application, status was {}", app.status);
        };
        assert_eq!(application.name, "PowerShell 7");
        assert_eq!(
            application.launch,
            LaunchTarget::Command {
                command: command.into(),
            }
        );
        assert_eq!(application.console_host, ConsoleHost::WindowsTerminal);
        assert_eq!(application.default_profile_id, profile_id);
        std::fs::remove_dir_all(root).unwrap();
    }
}
