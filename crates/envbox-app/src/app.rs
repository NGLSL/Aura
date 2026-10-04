//! EnvBox GUI application state and message update.
//! UI workflows are split into child modules; injection logic never lives here.

use envbox_core::{
    Application, AuditEvent, BrowserPrivacyProfile, ConsoleHost, EnvironmentProfile,
    InstanceStatus, LaunchTarget, LocaleProfile, RegistryProfile, TimezoneProfile,
};
use envbox_launcher::{format_args, parse_args, InstanceManager, RunTarget};
use envbox_storage::{
    enumerate_dynamic_timezone_ids, validate_application, validate_profile, windows_id_to_iana,
    ConfigStore,
};
use iced::{Subscription, Task};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

pub mod dns_editor;
mod picker;
mod update;
mod window;
pub mod workspace_management;
mod workspaces;

use crate::close_behavior::{self, CloseBehavior};
use crate::message::{ComboField, DnsChoice, LaunchKind, Message, Nav, StatusKind, WebRtcChoice};
use crate::options;
#[cfg(windows)]
use crate::tray::{TrayAction, TrayState};

#[derive(Clone, PartialEq, Eq)]
pub struct AppDraft {
    pub id: Option<Uuid>,
    pub name: String,
    pub kind: LaunchKind,
    pub path: String,
    pub args: String,
    pub work_dir: String,
    pub profile_id: Uuid,
    pub inherit: bool,
    pub console_host: ConsoleHost,
    pub audit: bool,
    /// Shell icon source (`path` or `path,index`) from picker / discovery.
    #[allow(dead_code)]
    pub icon_src: String,
}

impl AppDraft {
    pub fn blank(profile_id: Uuid) -> Self {
        Self {
            id: None,
            name: String::new(),
            kind: LaunchKind::Command,
            path: String::new(),
            args: String::new(),
            work_dir: String::new(),
            profile_id,
            inherit: true,
            console_host: ConsoleHost::Direct,
            audit: false,
            icon_src: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProfileDraft {
    pub id: Option<Uuid>,
    pub name: String,
    pub locale: String,
    pub ui: String,
    pub region: String,
    pub tz: String,
    pub tz_iana: String,
    pub dns_mode: DnsChoice,
    pub dns_editor: dns_editor::DnsEditor,
    pub webrtc: WebRtcChoice,
    pub env: String,
}

#[derive(Clone, Copy)]
pub enum RunSelection {
    Default,
    Profile(Uuid),
    Host,
}

#[derive(Clone, Copy)]
enum PendingNavigation {
    Workspace(Option<Uuid>),
    WorkspaceRefresh,
    Nav(Nav),
    App(Uuid),
    Profile(Uuid),
    NewApp,
    NewProfile,
    InstancesOf(Option<Uuid>),
    Exit { remember: bool },
}

pub struct EnvBoxApp {
    pub store: ConfigStore,
    pub nav: Nav,
    pub search: String,
    pub applications: Vec<Application>,
    pub profiles: Vec<EnvironmentProfile>,
    pub workspaces: workspaces::WorkspaceState,
    pub workspace_management: workspace_management::ManagementState,
    pub timezones: Vec<String>,
    pub instances: InstanceManager,
    pub app_draft: AppDraft,
    pub profile_draft: ProfileDraft,
    app_saved_draft: AppDraft,
    profile_saved_draft: ProfileDraft,
    pending_navigation: Option<PendingNavigation>,
    pub unsaved_error: Option<String>,
    pub app_edit_mode: bool,
    pub app_advanced: bool,
    pub profile_edit_mode: bool,
    pub profile_advanced: bool,
    pub instance_filter: Option<Uuid>,
    pub app_capabilities: HashMap<Uuid, crate::package::Capability>,
    resume_new_app: bool,
    pub audit_events: Vec<AuditEvent>,
    /// Empty = show all software.
    pub audit_filter: String,
    /// Open searchable-select field in the Profile editor.
    pub open_combo: Option<ComboField>,
    pub combo_query: String,
    pub status: String,
    pub status_kind: StatusKind,
    /// Explicit GitHub update workflow; no request is made at startup.
    pub update_checking: bool,
    pub update_status: Option<Result<crate::updater::CheckResult, String>>,
    pub update_asset: Option<crate::updater::InstallerAsset>,
    /// A verified installer waiting for the user to finish unsaved edits.
    pub update_installer_path: Option<PathBuf>,
    /// Local app picker (Kite-style discovery). `None` = closed.
    pub app_picker: Option<AppPickerState>,
    /// Cached icon PNGs for configured applications (uuid → png).
    pub app_icons: HashMap<Uuid, PathBuf>,
    pub close_behavior: CloseBehavior,
    pub close_dialog: bool,
    pub remember_close_choice: bool,
    pub close_error: Option<String>,
    #[cfg(windows)]
    tray: Option<TrayState>,
}

/// Local installed-app picker state.
#[derive(Debug, Clone, Default)]
pub struct AppPickerState {
    pub query: String,
    pub items: Vec<crate::discover::DiscoveredApp>,
    pub selected: Option<usize>,
    pub scanned: bool,
    pub loading: bool,
    pub icon_requests: HashSet<String>,
}

impl EnvBoxApp {
    pub fn new() -> (Self, Task<Message>) {
        let store = ConfigStore::new(ConfigStore::default_root());
        let _ = store.ensure_dirs();
        let applications = store
            .load_applications()
            .map(|d| d.applications)
            .unwrap_or_default();
        let profiles = store
            .load_profiles()
            .map(|d| d.profiles)
            .unwrap_or_default();
        let timezones = enumerate_dynamic_timezone_ids();
        let app_draft = AppDraft::blank(profiles.first().map(|p| p.id).unwrap_or_default());
        let tz = timezones
            .iter()
            .find(|t| t.eq_ignore_ascii_case("Pacific Standard Time"))
            .cloned()
            .or_else(|| timezones.first().cloned())
            .unwrap_or_default();
        let tz_iana = windows_id_to_iana(&tz).unwrap_or_default().to_string();
        let profile_draft = ProfileDraft {
            id: None,
            name: String::new(),
            locale: "en-US".into(),
            ui: "en-US".into(),
            region: "US".into(),
            tz,
            tz_iana,
            dns_mode: DnsChoice::Host,
            dns_editor: Default::default(),
            webrtc: WebRtcChoice::Host,
            env: String::new(),
        };
        let close_behavior = close_behavior::load(store.root());
        let workspaces = workspaces::WorkspaceState::load(
            &store,
            profiles
                .first()
                .map(|profile| profile.id)
                .unwrap_or_default(),
        );
        let app_capabilities = applications
            .iter()
            .map(|a| (a.id, capability_for_app(a)))
            .collect();
        let mut app = Self {
            store,
            nav: Nav::Apps,
            search: String::new(),
            applications,
            profiles,
            workspaces,
            workspace_management: Default::default(),
            timezones,
            instances: InstanceManager::new(),
            app_saved_draft: app_draft.clone(),
            profile_saved_draft: profile_draft.clone(),
            app_draft,
            profile_draft,
            pending_navigation: None,
            unsaved_error: None,
            app_edit_mode: false,
            app_advanced: false,
            profile_edit_mode: false,
            profile_advanced: false,
            instance_filter: None,
            app_capabilities,
            resume_new_app: false,
            audit_events: Vec::new(),
            audit_filter: String::new(),
            open_combo: None,
            combo_query: String::new(),
            status: String::new(),
            status_kind: StatusKind::Info,
            update_checking: false,
            update_status: None,
            update_asset: None,
            update_installer_path: None,
            app_picker: None,
            app_icons: HashMap::new(),
            close_behavior,
            close_dialog: false,
            remember_close_choice: false,
            close_error: None,
            #[cfg(windows)]
            tray: None,
        };
        app.load_audit();
        let icons_task = app.refresh_app_icons();
        if let Some(first) = app.applications.first() {
            let id = first.id;
            app.select_app(id);
        }
        let management_task = app.workspace_list();
        (app, Task::batch([icons_task, management_task]))
    }

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

    pub fn set_status(&mut self, kind: StatusKind, msg: impl Into<String>) {
        self.status_kind = kind;
        self.status = msg.into();
    }

    pub fn running_count(&self, id: Uuid) -> usize {
        self.instances
            .list()
            .into_iter()
            .filter(|inst| inst.application_id == id && inst.status == InstanceStatus::Running)
            .count()
    }

    pub fn unsaved_dialog_open(&self) -> bool {
        self.pending_navigation.is_some()
    }

    fn has_unsaved_edits(&self) -> bool {
        match self.nav {
            Nav::Workspaces => self.workspaces.dirty(),
            Nav::Apps => {
                self.app_edit_mode
                    && (self.app_draft != self.app_saved_draft
                        || (self.app_draft.id.is_none() && !self.app_draft.name.is_empty()))
            }
            Nav::Profiles => {
                self.profile_edit_mode && self.profile_draft != self.profile_saved_draft
            }
            _ => false,
        }
    }

    fn request_navigation(&mut self, action: PendingNavigation) -> Task<Message> {
        if self.has_unsaved_edits() {
            self.pending_navigation = Some(action);
            self.unsaved_error = None;
            Task::none()
        } else {
            self.perform_navigation(action)
        }
    }

    fn perform_navigation(&mut self, action: PendingNavigation) -> Task<Message> {
        match action {
            PendingNavigation::Workspace(id) => {
                self.workspace_management.selection_changed();
                self.workspaces.select(
                    id,
                    self.profiles
                        .first()
                        .map(|profile| profile.id)
                        .unwrap_or_default(),
                );
                self.workspace_list()
            }
            PendingNavigation::WorkspaceRefresh => {
                self.workspace_management.selection_changed();
                let selected = self.workspaces.selected;
                self.workspaces = workspaces::WorkspaceState::load(
                    &self.store,
                    self.profiles
                        .first()
                        .map(|profile| profile.id)
                        .unwrap_or_default(),
                );
                match self.store.load_profiles() {
                    Ok(doc) => self.profiles = doc.profiles,
                    Err(err) => self.workspaces.error = Some(err.to_string()),
                }
                if let Some(id) = selected.filter(|id| {
                    self.workspaces
                        .document
                        .containers
                        .iter()
                        .any(|value| value.id == *id)
                }) {
                    self.workspaces.select(
                        Some(id),
                        self.profiles
                            .first()
                            .map(|profile| profile.id)
                            .unwrap_or_default(),
                    );
                }
                match self.store.load_applications() {
                    Ok(doc) => self.applications = doc.applications,
                    Err(err) => self.workspaces.error = Some(err.to_string()),
                }
                self.workspace_list()
            }
            PendingNavigation::Nav(nav) => {
                let previous_nav = self.nav;
                self.nav = nav;
                if nav != Nav::Profiles {
                    self.resume_new_app = false;
                }
                if nav == Nav::Instances {
                    self.instance_filter = None;
                }
                if previous_nav != nav && nav == Nav::Profiles && self.profile_draft.id.is_none() {
                    if let Some(id) = self.profiles.first().map(|profile| profile.id) {
                        self.select_profile(id);
                    }
                }
                if previous_nav != nav && nav == Nav::Apps && self.app_draft.id.is_none() {
                    if let Some(id) = self.applications.first().map(|app| app.id) {
                        self.select_app(id);
                    }
                }
                if nav == Nav::Workspaces {
                    return self.workspace_list();
                }
                Task::none()
            }
            PendingNavigation::InstancesOf(id) => {
                self.instance_filter = id;
                self.nav = Nav::Instances;
                self.resume_new_app = false;
                Task::none()
            }
            PendingNavigation::App(id) => {
                self.select_app(id);
                Task::none()
            }
            PendingNavigation::Profile(id) => {
                self.select_profile(id);
                Task::none()
            }
            PendingNavigation::NewApp => self.begin_new_app(),
            PendingNavigation::NewProfile => {
                self.begin_new_profile();
                Task::none()
            }
            PendingNavigation::Exit { remember } => self.finish_exit(remember),
        }
    }

    fn begin_new_app(&mut self) -> Task<Message> {
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

    fn blank_profile(&self) -> ProfileDraft {
        let tz = self
            .timezones
            .iter()
            .find(|t| t.eq_ignore_ascii_case("Pacific Standard Time"))
            .cloned()
            .or_else(|| self.timezones.first().cloned())
            .unwrap_or_default();
        let tz_iana = windows_id_to_iana(&tz).unwrap_or_default().to_string();
        ProfileDraft {
            id: None,
            name: String::new(),
            locale: "en-US".into(),
            ui: "en-US".into(),
            region: "US".into(),
            tz,
            tz_iana,
            dns_mode: DnsChoice::Host,
            dns_editor: Default::default(),
            webrtc: WebRtcChoice::Host,
            env: String::new(),
        }
    }

    fn begin_new_profile(&mut self) {
        self.nav = Nav::Profiles;
        self.profile_draft = self.blank_profile();
        self.profile_saved_draft = self.profile_draft.clone();
        self.profile_edit_mode = true;
        self.profile_advanced = false;
        self.open_combo = None;
        self.combo_query.clear();
    }

    fn select_profile(&mut self, id: Uuid) {
        if let Some(profile) = self.profiles.iter().find(|profile| profile.id == id) {
            self.profile_draft = profile_to_draft(profile);
            self.profile_saved_draft = self.profile_draft.clone();
            self.profile_edit_mode = false;
            self.profile_advanced = false;
        }
        self.open_combo = None;
        self.combo_query.clear();
    }

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

    /// Options for a searchable select: common-first; query filters; Windows TZ search widens to host list.
    pub fn combo_options(&self, field: ComboField) -> Vec<String> {
        let q = self.combo_query.trim();
        let base = match field {
            ComboField::Region => options::common_regions(),
            ComboField::Locale | ComboField::Ui => options::common_locales(),
            ComboField::Timezone => {
                if q.is_empty() {
                    envbox_storage::common_windows_timezone_ids()
                        .into_iter()
                        .filter(|id| self.timezones.iter().any(|t| t.eq_ignore_ascii_case(id)))
                        .collect()
                } else {
                    self.timezones.clone()
                }
            }
            ComboField::TimezoneIana => envbox_storage::common_iana_timezone_ids(),
        };
        let cur = match field {
            ComboField::Region => self.profile_draft.region.as_str(),
            ComboField::Locale => self.profile_draft.locale.as_str(),
            ComboField::Ui => self.profile_draft.ui.as_str(),
            ComboField::Timezone => self.profile_draft.tz.as_str(),
            ComboField::TimezoneIana => self.profile_draft.tz_iana.as_str(),
        };
        let with_cur = options::with_current(&base, cur);
        let kind = match field {
            ComboField::Region => options::OptionKind::Region,
            ComboField::Locale | ComboField::Ui => options::OptionKind::Locale,
            ComboField::Timezone | ComboField::TimezoneIana => options::OptionKind::Timezone,
        };
        options::filter_options(&with_cur, q, kind)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn open_combo(&mut self, field: ComboField) {
        if self.open_combo == Some(field) {
            self.open_combo = None;
            self.combo_query.clear();
        } else {
            self.open_combo = Some(field);
            self.combo_query.clear();
        }
    }

    pub fn pick_combo(&mut self, field: ComboField, value: String) {
        match field {
            ComboField::Region => self.profile_draft.region = value,
            ComboField::Locale => self.profile_draft.locale = value,
            ComboField::Ui => self.profile_draft.ui = value,
            ComboField::Timezone => {
                self.profile_draft.tz_iana =
                    windows_id_to_iana(&value).unwrap_or_default().to_string();
                self.profile_draft.tz = value;
            }
            ComboField::TimezoneIana => {
                if let Some(win) = envbox_storage::iana_to_windows(&value) {
                    self.profile_draft.tz = win.to_string();
                }
                self.profile_draft.tz_iana = value;
            }
        }
        self.open_combo = None;
        self.combo_query.clear();
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

    pub fn profile_name(&self, id: Uuid) -> String {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".into())
    }

    pub fn view(&self) -> iced::Element<'_, Message> {
        crate::views::view(self)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::window::close_requests().map(Message::WindowCloseRequested),
            #[cfg(windows)]
            crate::singleton::subscription(),
            if self.tray.is_some() {
                iced::time::every(std::time::Duration::from_millis(200))
                    .map(|_| Message::WindowTrayPoll)
            } else {
                Subscription::none()
            },
        ])
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Nav(n) => {
                if n != self.nav {
                    if self.update_checking {
                        self.set_status(StatusKind::Info, "更新正在进行，请等待完成后再切换页面");
                        return Task::none();
                    }
                    return self.request_navigation(PendingNavigation::Nav(n));
                }
            }
            Message::Search(v) => self.search = v,
            Message::WorkspaceNew => {
                return self.request_navigation(PendingNavigation::Workspace(None));
            }
            Message::WorkspaceSelect(id) => {
                return self.request_navigation(PendingNavigation::Workspace(Some(id)));
            }
            Message::WorkspaceRefresh => {
                return self.request_navigation(PendingNavigation::WorkspaceRefresh);
            }
            Message::WorkspaceName(value) => self.workspaces.draft.name = value,
            Message::WorkspaceProfile(id) => self.workspaces.draft.profile_id = id,
            Message::WorkspaceCancel => self.workspaces.discard(),
            Message::WorkspaceSave => return self.save_workspace(),
            Message::WorkspaceApplication(id) => {
                self.workspace_management.application_id = Some(id)
            }
            Message::WorkspaceRun => return self.workspace_run(),
            Message::WorkspaceList => return self.workspace_list(),
            Message::WorkspaceStop(id) => return self.workspace_stop(Some(id)),
            Message::WorkspaceStopAll => return self.workspace_stop(None),
            Message::WorkspaceRunStatus => return self.workspace_run_status(),
            Message::WorkspaceManagementResult(value) => {
                return self.finish_workspace_management(value)
            }
            Message::WorkspaceRuleTarget(value) => self.workspaces.rule_draft.target = value,
            Message::WorkspaceRuleAction(value) => self.workspaces.rule_draft.action = value,
            Message::WorkspaceRulePath(value) => self.workspaces.rule_draft.path = value,
            Message::WorkspaceRuleAdd => {
                if let Err(err) = self.workspaces.add_rule(&self.store) {
                    self.workspaces.error = Some(err);
                }
            }
            Message::WorkspaceRuleRemove(index) => {
                if index < self.workspaces.draft.storage_policy.rules.len() {
                    self.workspaces.draft.storage_policy.rules.remove(index);
                }
            }
            Message::AppNew => return self.request_navigation(PendingNavigation::NewApp),
            Message::AppPickerQuery(q) => {
                if let Some(p) = self.app_picker.as_mut() {
                    p.query = q;
                }
                let first = self.filtered_picker_items().first().map(|(idx, _)| *idx);
                if let Some(p) = self.app_picker.as_mut() {
                    p.selected = first;
                }
                return self.extract_picker_icons();
            }
            Message::AppPickerLoaded(items) => {
                let n = items.len();
                if let Some(p) = self.app_picker.as_mut() {
                    p.items = items;
                    p.loading = false;
                    p.scanned = true;
                }
                let first = self.filtered_picker_items().first().map(|(idx, _)| *idx);
                if let Some(p) = self.app_picker.as_mut() {
                    p.selected = first;
                }
                self.set_status(StatusKind::Info, format!("已发现 {n} 个本机入口"));
                return self.extract_picker_icons();
            }
            Message::AppPickerIcons(pairs) => {
                if let Some(p) = self.app_picker.as_mut() {
                    let map: HashMap<String, std::path::PathBuf> = pairs.into_iter().collect();
                    for item in p.items.iter_mut() {
                        if item.icon_png.is_none() {
                            if let Some(png) = map.get(&item.icon_key()) {
                                item.icon_png = Some(png.clone());
                            }
                        }
                    }
                }
            }
            Message::AppIconsLoaded(pairs) => {
                for (id, png) in pairs {
                    self.app_icons.insert(id, png);
                }
            }
            Message::AppPickerSelect(i) => {
                if let Some(p) = self.app_picker.as_mut() {
                    p.selected = Some(i);
                }
            }
            Message::AppPickerUseSelected => {
                return self.apply_picker_selection();
            }
            Message::AppPickerManual => {
                self.app_picker = None;
                self.app_draft =
                    AppDraft::blank(self.profiles.first().map(|p| p.id).unwrap_or_default());
                self.app_saved_draft = self.app_draft.clone();
                self.app_edit_mode = true;
                self.app_advanced = false;
            }
            Message::AppPickerClose => {
                self.app_picker = None;
            }
            Message::AppName(v) => self.app_draft.name = v,
            Message::AppLaunchKind(k) => self.app_draft.kind = k,
            Message::AppPath(v) => self.app_draft.path = v,
            Message::AppArgs(v) => self.app_draft.args = v,
            Message::AppWorkDir(v) => self.app_draft.work_dir = v,
            Message::AppProfile(id) => self.app_draft.profile_id = id,
            Message::AppInherit(v) => self.app_draft.inherit = v,
            Message::AppConsoleHost(v) => self.app_draft.console_host = v,
            Message::AppAudit(v) => self.app_draft.audit = v,
            Message::AppSelect(id) => {
                if self.app_draft.id != Some(id) {
                    return self.request_navigation(PendingNavigation::App(id));
                }
            }
            Message::AppEdit => {
                self.app_saved_draft = self.app_draft.clone();
                self.app_edit_mode = true;
                self.app_advanced = false;
            }
            Message::AppEditCancel => {
                self.app_draft = self.app_saved_draft.clone();
                self.app_edit_mode = false;
                self.app_advanced = false;
                if self.app_draft.id.is_none() {
                    if let Some(id) = self.applications.first().map(|app| app.id) {
                        self.select_app(id);
                    }
                }
            }
            Message::AppAdvancedToggle => self.app_advanced = !self.app_advanced,
            Message::AppOpenLocation => {
                if let Some(id) = self.app_draft.id {
                    self.open_app_location(id);
                }
            }
            Message::AppOpenLocationId(id) => self.open_app_location(id),
            Message::AppBrowseWorkDir => {
                let target = if !self.app_draft.work_dir.trim().is_empty() {
                    self.app_draft.work_dir.trim().to_string()
                } else {
                    ".".to_string()
                };
                let _ = std::process::Command::new("explorer").arg(&target).spawn();
            }
            Message::AppSave => return self.save_app(),
            Message::AppDelete => return self.delete_app(),
            Message::AppRunId(id) => return self.run_app(id, RunSelection::Default),
            Message::AppRunWithId(id, profile_id) => {
                let selection = profile_id
                    .map(RunSelection::Profile)
                    .unwrap_or(RunSelection::Host);
                return self.run_app(id, selection);
            }
            Message::ProfileName(v) => self.profile_draft.name = v,
            Message::ProfileLocale(v) => self.profile_draft.locale = v,
            Message::ProfileUi(v) => self.profile_draft.ui = v,
            Message::ProfileRegion(v) => self.profile_draft.region = v,
            Message::ProfileTz(v) => {
                self.profile_draft.tz_iana = windows_id_to_iana(&v).unwrap_or_default().to_string();
                self.profile_draft.tz = v;
            }
            Message::ProfileTzIana(v) => self.profile_draft.tz_iana = v,
            Message::ProfileDnsMode(m) => self.profile_draft.dns_mode = m,
            Message::ProfileDnsStrict(value) => self.profile_draft.dns_editor.strict = value,
            Message::ProfileDnsTransport(value) => {
                self.profile_draft.dns_editor.draft = dns_editor::UpstreamDraft::default();
                self.profile_draft.dns_editor.draft.transport = value;
                if value == dns_editor::TransportChoice::Dot {
                    self.profile_draft.dns_editor.draft.port = "853".into();
                }
            }
            Message::ProfileDnsAddress(value) => {
                self.profile_draft.dns_editor.draft.address = value
            }
            Message::ProfileDnsPort(value) => self.profile_draft.dns_editor.draft.port = value,
            Message::ProfileDnsServerName(value) => {
                self.profile_draft.dns_editor.draft.server_name = value
            }
            Message::ProfileDnsUrl(value) => self.profile_draft.dns_editor.draft.url = value,
            Message::ProfileDnsBootstrap(value) => {
                self.profile_draft.dns_editor.draft.bootstrap = value
            }
            Message::ProfileDnsAdd => {
                if let Err(err) = self.profile_draft.dns_editor.add() {
                    self.set_status(StatusKind::Error, format!("上游配置失败：{err}"));
                }
            }
            Message::ProfileDnsRemove(index) => {
                if index < self.profile_draft.dns_editor.upstreams.len() {
                    self.profile_draft.dns_editor.upstreams.remove(index);
                }
            }
            Message::ProfileDnsMove(index, upward) => {
                self.profile_draft.dns_editor.move_upstream(index, upward)
            }
            Message::ProfileWebRtc(c) => self.profile_draft.webrtc = c,
            Message::ProfileEnv(v) => self.profile_draft.env = v,
            Message::ComboToggle(field) => self.open_combo(field),
            Message::ComboQuery(q) => self.combo_query = q,
            Message::ComboPick(field, v) => self.pick_combo(field, v),
            Message::ComboClose => {
                self.open_combo = None;
                self.combo_query.clear();
            }
            Message::ProfileSelect(id) => {
                if self.profile_draft.id != Some(id) {
                    return self.request_navigation(PendingNavigation::Profile(id));
                }
            }
            Message::ProfileEdit => {
                self.profile_saved_draft = self.profile_draft.clone();
                self.profile_edit_mode = true;
                self.profile_advanced = false;
            }
            Message::ProfileEditCancel => {
                self.profile_draft = self.profile_saved_draft.clone();
                self.profile_edit_mode = false;
                self.profile_advanced = false;
                self.resume_new_app = false;
                self.open_combo = None;
                self.combo_query.clear();
                if self.profile_draft.id.is_none() {
                    if let Some(id) = self.profiles.first().map(|profile| profile.id) {
                        self.select_profile(id);
                    }
                }
            }
            Message::ProfileAdvancedToggle => self.profile_advanced = !self.profile_advanced,
            Message::ProfileSave => return self.save_profile(),
            Message::ProfileDelete => return self.delete_profile(),
            Message::ProfileNew => return self.request_navigation(PendingNavigation::NewProfile),
            Message::InstanceRefresh => {
                self.instances.refresh_all();
                self.set_status(StatusKind::Success, "实例已刷新");
            }
            Message::InstanceFilter(id) => {
                return self.request_navigation(PendingNavigation::InstancesOf(id));
            }
            Message::InstanceStop(id) => {
                if let Err(err) = self.instances.stop(id) {
                    self.set_status(StatusKind::Error, format!("停止失败: {err}"));
                } else {
                    self.set_status(StatusKind::Success, "已停止");
                }
            }
            Message::AuditRefresh => {
                self.load_audit();
                self.set_status(
                    StatusKind::Success,
                    format!("已加载 {} 条审计事件", self.audit_events.len()),
                );
            }
            Message::AuditFilter(v) => self.audit_filter = v,
            Message::OpenAuditDir => {
                let _ = self.store.ensure_audit_dir();
                let dir = self.store.audit_dir();
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                self.set_status(StatusKind::Info, "已在资源管理器中打开审计日志目录");
            }
            Message::OpenConfigDir => {
                let _ = self.store.ensure_dirs();
                let root = self.store.root();
                let _ = std::process::Command::new("explorer").arg(root).spawn();
                self.set_status(StatusKind::Info, "已在资源管理器中打开配置目录");
            }
            Message::RunProbe => {
                let mut probe_path = None;
                if let Ok(cur) = std::env::current_exe() {
                    if let Some(dir) = cur.parent() {
                        for cand in [
                            dir.join("envbox-probe.exe"),
                            dir.join("target").join("debug").join("envbox-probe.exe"),
                            dir.join("target").join("release").join("envbox-probe.exe"),
                        ] {
                            if cand.exists() {
                                probe_path = Some(cand);
                                break;
                            }
                        }
                    }
                }
                if probe_path.is_none() {
                    for cand in &[
                        "target\\debug\\envbox-probe.exe",
                        "target\\release\\envbox-probe.exe",
                        ".\\envbox-probe.exe",
                    ] {
                        let p = std::path::PathBuf::from(cand);
                        if p.exists() {
                            probe_path = Some(p);
                            break;
                        }
                    }
                }
                if let Some(p) = probe_path {
                    let probe_str = p.display().to_string();
                    let spawn_res = std::process::Command::new("cmd.exe")
                        .args([
                            "/C",
                            "start",
                            "Aura · EnvBox Probe",
                            "cmd.exe",
                            "/K",
                            &probe_str,
                        ])
                        .spawn();
                    if spawn_res.is_ok() {
                        self.set_status(StatusKind::Success, "已在独立控制台中运行环境探针");
                    } else {
                        self.set_status(StatusKind::Error, "运行环境探针启动失败");
                    }
                } else {
                    self.set_status(
                        StatusKind::Error,
                        "未找到 envbox-probe.exe，请先通过 cargo build 构建探针",
                    );
                }
            }
            Message::CheckUpdate => return self.start_update_check(),
            Message::UpdateResult(result) => return self.finish_update_check(result),
            Message::InstallUpdate => return self.start_update_install(),
            Message::UpdateDownloadResult(result) => return self.finish_update_download(result),
            Message::UpdateInstallResult(result) => return self.finish_update_install(result),
            Message::OpenReleases => return self.open_releases(),
            Message::OpenRepository => return self.open_repository(),
            Message::StatusDismiss => self.status.clear(),
            Message::UnsavedCancel => {
                self.pending_navigation = None;
                self.unsaved_error = None;
            }
            Message::UnsavedDiscard => {
                let Some(action) = self.pending_navigation.take() else {
                    return Task::none();
                };
                self.unsaved_error = None;
                match self.nav {
                    Nav::Workspaces => self.workspaces.discard(),
                    Nav::Apps => {
                        self.app_draft = self.app_saved_draft.clone();
                        self.app_edit_mode = false;
                    }
                    Nav::Profiles => {
                        self.profile_draft = self.profile_saved_draft.clone();
                        self.profile_edit_mode = false;
                    }
                    _ => {}
                }
                return self.perform_navigation(action);
            }
            Message::UnsavedSave => {
                let Some(action) = self.pending_navigation.take() else {
                    return Task::none();
                };
                // The pending navigation takes precedence over the first-run
                // "create a Profile, then add an app" continuation.
                self.resume_new_app = false;
                let save_task = match self.nav {
                    Nav::Workspaces => self.save_workspace(),
                    Nav::Apps => self.save_app(),
                    Nav::Profiles => self.save_profile(),
                    _ => Task::none(),
                };
                if self.has_unsaved_edits() {
                    self.pending_navigation = Some(action);
                    self.unsaved_error = Some(self.status.clone());
                    return save_task;
                }
                self.unsaved_error = None;
                return Task::batch([save_task, self.perform_navigation(action)]);
            }
            Message::WindowDrag => {
                return iced::window::get_latest()
                    .then(|id| id.map(iced::window::drag).unwrap_or_else(Task::none))
            }
            Message::WindowMinimize => {
                return iced::window::get_latest().then(|id| {
                    id.map(|id| iced::window::minimize(id, true))
                        .unwrap_or_else(Task::none)
                })
            }
            Message::WindowToggleMaximize => {
                return iced::window::get_latest().then(|id| {
                    id.map(iced::window::toggle_maximize)
                        .unwrap_or_else(Task::none)
                })
            }
            Message::WindowClose | Message::WindowCloseRequested(_) => return self.request_close(),
            Message::WindowRememberChoice(remember) => self.remember_close_choice = remember,
            Message::WindowCloseCancel => {
                self.close_dialog = false;
                self.remember_close_choice = false;
                self.close_error = None;
            }
            Message::WindowCloseToTray => return self.hide_to_tray(),
            Message::WindowExit => return self.request_exit(),
            Message::WindowClosePreferenceReset => {
                match close_behavior::save(self.store.root(), CloseBehavior::Ask) {
                    Ok(()) => {
                        self.close_behavior = CloseBehavior::Ask;
                        self.remember_close_choice = false;
                        self.set_status(StatusKind::Success, "下次关闭 Aura 时将重新询问");
                    }
                    Err(err) => {
                        self.set_status(StatusKind::Error, format!("保存关闭设置失败：{err}"))
                    }
                }
            }
            Message::WindowTrayPoll =>
            {
                #[cfg(windows)]
                if let Some(tray) = &self.tray {
                    match tray.poll() {
                        Some(TrayAction::Restore) => return self.update(Message::WindowRestore),
                        Some(TrayAction::Exit) => {
                            let exit = self.request_exit();
                            if self.pending_navigation.is_some() {
                                return self.update(Message::WindowRestore);
                            }
                            return exit;
                        }
                        None => {}
                    }
                }
            }
            Message::WindowRestore => {
                #[cfg(windows)]
                {
                    let was_hidden = self.tray.take().is_some();
                    return iced::window::get_latest().then(move |id| {
                        id.map(|id| {
                            let reveal = if was_hidden {
                                iced::window::change_mode(id, iced::window::Mode::Windowed)
                            } else {
                                Task::none()
                            };
                            reveal
                                .chain(iced::window::minimize(id, false))
                                .chain(iced::window::gain_focus(id))
                        })
                        .unwrap_or_else(Task::none)
                    });
                }
            }
        }
        Task::none()
    }

    fn save_app(&mut self) -> Task<Message> {
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

    fn delete_app(&mut self) -> Task<Message> {
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

    fn run_app(&mut self, app_id: Uuid, sel: RunSelection) -> Task<Message> {
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

    fn open_app_location(&mut self, app_id: Uuid) {
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

    fn save_profile(&mut self) -> Task<Message> {
        let dns = match self
            .profile_draft
            .dns_editor
            .to_profile(self.profile_draft.dns_mode.to_mode())
        {
            Ok(dns) => dns,
            Err(err) => {
                self.set_status(StatusKind::Error, format!("保存失败：{err}"));
                return Task::none();
            }
        };
        let mut environment = HashMap::new();
        for line in self
            .profile_draft
            .env
            .split(|c| c == ';' || c == '\n' || c == '\r')
        {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                self.set_status(
                    StatusKind::Error,
                    format!("保存失败：环境变量需要 KEY=VALUE，收到 {line:?}"),
                );
                return Task::none();
            };
            let k = k.trim();
            if k.is_empty() {
                self.set_status(
                    StatusKind::Error,
                    format!("保存失败：环境变量键为空 {line:?}"),
                );
                return Task::none();
            }
            environment.insert(k.to_string(), v.trim().to_string());
        }
        let profile = EnvironmentProfile {
            id: self.profile_draft.id.unwrap_or_else(Uuid::new_v4),
            name: self.profile_draft.name.clone(),
            locale: LocaleProfile {
                locale_name: self.profile_draft.locale.clone(),
                ui_language: self.profile_draft.ui.clone(),
                region: self.profile_draft.region.clone(),
            },
            timezone: TimezoneProfile {
                windows_id: self.profile_draft.tz.clone(),
                iana_id: self.profile_draft.tz_iana.trim().to_string(),
            },
            dns,
            environment,
            registry: RegistryProfile::default(),
            browser: BrowserPrivacyProfile {
                webrtc: self.profile_draft.webrtc.to_policy(),
            },
        };
        if let Err(err) = validate_profile(&profile) {
            self.set_status(StatusKind::Error, format!("保存失败: {err}"));
            return Task::none();
        }
        let mut doc = match self.store.load_profiles() {
            Ok(doc) => doc,
            Err(err) => {
                self.set_status(StatusKind::Error, format!("读取配置失败：{err}"));
                return Task::none();
            }
        };
        if let Some(slot) = doc.profiles.iter_mut().find(|p| p.id == profile.id) {
            *slot = profile.clone();
        } else {
            doc.profiles.push(profile.clone());
        }
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                self.profile_draft.id = Some(profile.id);
                self.profile_saved_draft = self.profile_draft.clone();
                self.profile_edit_mode = false;
                self.set_status(StatusKind::Success, "环境配置已保存");
                if self.resume_new_app {
                    self.resume_new_app = false;
                    return self.begin_new_app();
                }
            }
            Err(err) => self.set_status(StatusKind::Error, format!("保存失败: {err}")),
        }
        Task::none()
    }

    fn delete_profile(&mut self) -> Task<Message> {
        let Some(id) = self.profile_draft.id else {
            self.set_status(StatusKind::Error, "请先选择环境配置");
            return Task::none();
        };
        let in_use = self
            .applications
            .iter()
            .filter(|app| app.default_profile_id == id)
            .count();
        if in_use > 0 {
            self.set_status(
                StatusKind::Error,
                format!("有 {in_use} 个应用使用此环境配置，请先更改这些应用的默认环境"),
            );
            return Task::none();
        }
        let mut doc = self.store.load_profiles().unwrap_or_default();
        doc.profiles.retain(|p| p.id != id);
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                if let Some(next_id) = self.profiles.first().map(|profile| profile.id) {
                    self.select_profile(next_id);
                } else {
                    self.profile_draft = self.blank_profile();
                    self.profile_saved_draft = self.profile_draft.clone();
                    self.profile_edit_mode = false;
                }
                self.set_status(StatusKind::Success, "环境配置已删除");
            }
            Err(err) => self.set_status(StatusKind::Error, format!("删除失败: {err}")),
        }
        Task::none()
    }
}

fn capability_for_app(app: &Application) -> crate::package::Capability {
    let target = match &app.launch {
        LaunchTarget::Command { command } => command.as_str(),
        LaunchTarget::Executable { path } => path.to_str().unwrap_or_default(),
        LaunchTarget::Packaged { aumid, .. } => aumid.as_str(),
    };
    crate::package::classify_target(target, &format_args(&app.arguments))
}

pub fn profile_to_draft(p: &EnvironmentProfile) -> ProfileDraft {
    ProfileDraft {
        id: Some(p.id),
        name: p.name.clone(),
        locale: p.locale.locale_name.clone(),
        ui: p.locale.ui_language.clone(),
        region: p.locale.region.clone(),
        tz: p.timezone.windows_id.clone(),
        tz_iana: p.timezone.iana_id.clone(),
        dns_mode: DnsChoice::from_mode(&p.dns.mode),
        dns_editor: dns_editor::DnsEditor::from_profile(&p.dns),
        webrtc: WebRtcChoice::from_policy(&p.browser.webrtc),
        env: p
            .environment
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(";"),
    }
}

pub fn status_label(s: InstanceStatus) -> &'static str {
    match s {
        InstanceStatus::Starting => "Starting",
        InstanceStatus::Running => "Running",
        InstanceStatus::Stopping => "Stopping",
        InstanceStatus::Exited => "Exited",
        InstanceStatus::Failed => "Failed",
    }
}
