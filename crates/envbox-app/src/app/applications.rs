//! Application editing, persistence, launch, and target capabilities.

use super::{AppDraft, AppPickerState, EnvBoxApp, RunSelection};
use crate::message::{LaunchKind, Message, Nav, StatusKind};
use envbox_core::{Application, LaunchTarget};
use envbox_launcher::{format_args, parse_args, RunTarget};
use envbox_storage::validate_application;
use iced::Task;
use std::path::PathBuf;
use uuid::Uuid;

impl EnvBoxApp {
    pub fn select_app(&mut self, id: Uuid) {
        if let Some(app) = self.applications.iter().find(|a| a.id == id) {
            self.app_draft = AppDraft {
                id: Some(app.id),
                name: app.name.clone(),
                kind: match &app.launch {
                    LaunchTarget::Command { .. } => LaunchKind::Command,
                    LaunchTarget::Executable { .. } => LaunchKind::Executable,
                    LaunchTarget::Packaged { .. } => LaunchKind::Packaged,
                },
                path: match &app.launch {
                    LaunchTarget::Command { command } => command.clone(),
                    LaunchTarget::Executable { path } => path.display().to_string(),
                    LaunchTarget::Packaged { aumid, .. } => aumid.clone(),
                },
                args: format_args(&app.arguments),
                work_dir: app
                    .working_directory
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                profile_id: app.default_profile_id,
                inherit: app.inherit_children,
                console_host: app.console_host,
                audit: app.audit,
                icon_src: match &app.launch {
                    LaunchTarget::Command { command } => command.clone(),
                    LaunchTarget::Executable { path } => path.display().to_string(),
                    LaunchTarget::Packaged { aumid, .. } => aumid.clone(),
                },
            };
            self.app_saved_draft = self.app_draft.clone();
            self.app_edit_mode = false;
            self.app_advanced = false;
        }
    }

    pub(super) fn begin_new_app(&mut self) -> Task<Message> {
        if self.profiles.is_empty() {
            self.resume_new_app = true;
            self.begin_new_profile();
            self.set_status(StatusKind::Info, "先创建环境配置，保存后继续添加应用");
            return Task::none();
        }
        self.nav = Nav::Apps;
        self.app_picker = Some(AppPickerState::default());
        self.scan_apps_for_picker()
    }

    pub fn filtered_apps(&self) -> Vec<&Application> {
        let q = self.search.trim().to_ascii_lowercase();
        self.applications
            .iter()
            .filter(|a| {
                q.is_empty()
                    || a.name.to_ascii_lowercase().contains(&q)
                    || match &a.launch {
                        LaunchTarget::Command { command } => {
                            command.to_ascii_lowercase().contains(&q)
                        }
                        LaunchTarget::Executable { path } => {
                            path.to_string_lossy().to_ascii_lowercase().contains(&q)
                        }
                        LaunchTarget::Packaged { aumid, .. } => {
                            aumid.to_ascii_lowercase().contains(&q)
                        }
                    }
            })
            .collect()
    }

    pub(super) fn save_app(&mut self) -> Task<Message> {
        if self.profiles.is_empty() {
            self.set_status(StatusKind::Error, "保存失败：请先创建环境配置");
            return Task::none();
        }
        if !self
            .profiles
            .iter()
            .any(|p| p.id == self.app_draft.profile_id)
        {
            self.set_status(StatusKind::Error, "保存失败：默认环境配置不存在");
            return Task::none();
        }
        // AUMID / shell:AppsFolder / WindowsApps / execution-alias paths must
        // become LaunchTarget::Packaged — never a CreateProcess Executable
        // (already-saved legacy entries also normalize at run).
        let raw = match self.app_draft.kind {
            LaunchKind::Command => LaunchTarget::Command {
                command: self.app_draft.path.clone(),
            },
            LaunchKind::Executable | LaunchKind::Packaged => LaunchTarget::Executable {
                path: PathBuf::from(self.app_draft.path.clone()),
            },
        };
        let launch = envbox_launcher::normalize_launch_target(&raw);
        let arguments = parse_args(&self.app_draft.args);
        let working_directory = if self.app_draft.work_dir.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(self.app_draft.work_dir.trim()))
        };
        let app = Application {
            id: self.app_draft.id.unwrap_or_else(Uuid::new_v4),
            name: self.app_draft.name.clone(),
            launch,
            working_directory,
            arguments,
            default_profile_id: self.app_draft.profile_id,
            inherit_children: self.app_draft.inherit,
            console_host: self.app_draft.console_host,
            audit: self.app_draft.audit,
        };
        if let Err(err) = validate_application(&app) {
            self.set_status(StatusKind::Error, format!("保存失败: {err}"));
            return Task::none();
        }
        let mut doc = self.store.load_applications().unwrap_or_default();
        if let Some(slot) = doc.applications.iter_mut().find(|a| a.id == app.id) {
            *slot = app.clone();
        } else {
            doc.applications.push(app.clone());
        }
        match self.store.save_applications(&doc) {
            Ok(()) => {
                self.applications = doc.applications;
                self.app_draft.id = Some(app.id);
                self.app_saved_draft = self.app_draft.clone();
                self.app_edit_mode = false;
                self.app_capabilities
                    .insert(app.id, capability_for_app(&app));
                self.set_status(StatusKind::Success, "应用已保存");
                return self.refresh_app_icons();
            }
            Err(err) => self.set_status(StatusKind::Error, format!("保存失败: {err}")),
        }
        Task::none()
    }

    pub(super) fn delete_app(&mut self) -> Task<Message> {
        let Some(id) = self.app_draft.id else {
            self.set_status(StatusKind::Error, "请先选择应用");
            return Task::none();
        };
        let mut doc = self.store.load_applications().unwrap_or_default();
        doc.applications.retain(|a| a.id != id);
        match self.store.save_applications(&doc) {
            Ok(()) => {
                self.applications = doc.applications;
                self.app_capabilities.remove(&id);
                if let Some(next_id) = self.applications.first().map(|app| app.id) {
                    self.select_app(next_id);
                } else {
                    self.app_draft = AppDraft::blank(
                        self.profiles
                            .first()
                            .map(|profile| profile.id)
                            .unwrap_or_default(),
                    );
                    self.app_saved_draft = self.app_draft.clone();
                    self.app_edit_mode = false;
                }
                self.set_status(StatusKind::Success, "应用已删除");
            }
            Err(err) => self.set_status(StatusKind::Error, format!("删除失败: {err}")),
        }
        Task::none()
    }

    pub(super) fn run_app(&mut self, app_id: Uuid, sel: RunSelection) -> Task<Message> {
        let Some(app) = self.applications.iter().find(|a| a.id == app_id).cloned() else {
            self.set_status(StatusKind::Error, "应用不存在");
            return Task::none();
        };
        if !matches!(sel, RunSelection::Host) {
            if let Some(capability) = self.app_capabilities.get(&app_id) {
                if capability.injection == crate::package::InjectionSupport::Unsupported {
                    self.set_status(
                        StatusKind::Error,
                        format!(
                            "「{}」无法使用环境配置启动。{}",
                            app.name,
                            capability.user_explanation()
                        ),
                    );
                    return Task::none();
                }
            }
        }
        let target = match sel {
            RunSelection::Host => RunTarget::Host,
            RunSelection::Default => {
                let Some(profile) = self
                    .profiles
                    .iter()
                    .find(|p| p.id == app.default_profile_id)
                    .cloned()
                else {
                    self.set_status(StatusKind::Error, "环境配置不存在");
                    return Task::none();
                };
                RunTarget::Profile(profile)
            }
            RunSelection::Profile(id) => {
                let Some(profile) = self.profiles.iter().find(|p| p.id == id).cloned() else {
                    self.set_status(StatusKind::Error, "环境配置不存在");
                    return Task::none();
                };
                RunTarget::Profile(profile)
            }
        };
        match self.instances.run(&app, target) {
            Ok(id) => {
                self.set_status(StatusKind::Success, format!("已启动实例 {id}"));
                self.nav = Nav::Apps;
                self.instances.refresh_all();
            }
            Err(err) => self.set_status(StatusKind::Error, format!("启动失败: {err}")),
        }
        Task::none()
    }

    pub(super) fn open_app_location(&mut self, app_id: Uuid) {
        let Some(app) = self.applications.iter().find(|app| app.id == app_id) else {
            self.set_status(StatusKind::Error, "应用不存在");
            return;
        };
        let LaunchTarget::Executable { path } = &app.launch else {
            self.set_status(StatusKind::Info, "此启动方式没有可打开的程序文件位置");
            return;
        };
        let Some(folder) = path.parent().filter(|parent| parent.is_dir()) else {
            self.set_status(StatusKind::Error, "程序所在目录不存在");
            return;
        };
        if let Err(err) = std::process::Command::new("explorer.exe")
            .arg(folder)
            .spawn()
        {
            self.set_status(StatusKind::Error, format!("打开程序位置失败：{err}"));
        }
    }
}

pub(super) fn capability_for_app(app: &Application) -> crate::package::Capability {
    let target = match &app.launch {
        LaunchTarget::Command { command } => command.as_str(),
        LaunchTarget::Executable { path } => path.to_str().unwrap_or_default(),
        LaunchTarget::Packaged { aumid, .. } => aumid.as_str(),
    };
    crate::package::classify_target(target, &format_args(&app.arguments))
}
