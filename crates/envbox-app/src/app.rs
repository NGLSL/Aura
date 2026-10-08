//! EnvBox GUI application state and message update.
//! UI workflows are split into child modules; injection logic never lives here.

use envbox_core::{Application, AuditEvent, ConsoleHost, EnvironmentProfile, InstanceStatus};
use envbox_launcher::InstanceManager;
use envbox_storage::{enumerate_dynamic_timezone_ids, windows_id_to_iana, ConfigStore};
use iced::{Subscription, Task};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

#[path = "app/applications.rs"]
mod applications;
#[path = "app/audit.rs"]
mod audit;
#[path = "app/dns_editor.rs"]
pub mod dns_editor;
#[path = "app/identity_editor.rs"]
pub mod identity_editor;
#[path = "app/navigation.rs"]
mod navigation;
#[path = "app/picker.rs"]
mod picker;
#[path = "app/profiles.rs"]
mod profiles;
#[path = "app/update.rs"]
mod update;
#[path = "app/window.rs"]
mod window;
#[path = "app/workspace_management.rs"]
pub mod workspace_management;
#[path = "app/workspaces.rs"]
mod workspaces;

use crate::close_behavior::{self, CloseBehavior};
use crate::message::{ComboField, DnsChoice, LaunchKind, Message, Nav, StatusKind, WebRtcChoice};
#[cfg(windows)]
use crate::tray::{TrayAction, TrayState};
use applications::capability_for_app;
#[cfg(test)]
pub use profiles::profile_to_draft;

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
    pub identity: identity_editor::IdentityDraft,
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
    pub content_panes: iced::widget::pane_grid::State<bool>,
    pub profile_panes: iced::widget::pane_grid::State<bool>,
    pane_ratios: close_behavior::PaneRatios,
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
    pub audit_total: usize,
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
        Self::new_with_store(store)
    }

    fn new_with_store(store: ConfigStore) -> (Self, Task<Message>) {
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
            identity: Default::default(),
            env: String::new(),
        };
        let close_behavior = close_behavior::load(store.root());
        let pane_ratios = close_behavior::load_pane_ratios(store.root());
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
            content_panes: iced::widget::pane_grid::State::with_configuration(
                iced::widget::pane_grid::Configuration::Split {
                    axis: iced::widget::pane_grid::Axis::Vertical,
                    ratio: pane_ratios.apps,
                    a: Box::new(iced::widget::pane_grid::Configuration::Pane(false)),
                    b: Box::new(iced::widget::pane_grid::Configuration::Pane(true)),
                },
            ),
            profile_panes: iced::widget::pane_grid::State::with_configuration(
                iced::widget::pane_grid::Configuration::Split {
                    axis: iced::widget::pane_grid::Axis::Vertical,
                    ratio: pane_ratios.profiles,
                    a: Box::new(iced::widget::pane_grid::Configuration::Pane(false)),
                    b: Box::new(iced::widget::pane_grid::Configuration::Pane(true)),
                },
            ),
            search: String::new(),
            pane_ratios,
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
            audit_total: 0,
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
            Message::ContentPaneResized(event) => {
                let (panes, saved_ratio, minimum) = if self.nav == Nav::Profiles {
                    (
                        &mut self.profile_panes,
                        &mut self.pane_ratios.profiles,
                        0.32,
                    )
                } else {
                    (&mut self.content_panes, &mut self.pane_ratios.apps, 0.35)
                };
                if event.ratio.is_finite() {
                    let ratio = event.ratio.clamp(minimum, 0.75);
                    panes.resize(event.split, ratio);
                    if *saved_ratio != ratio {
                        *saved_ratio = ratio;
                        if let Err(error) =
                            close_behavior::save_pane_ratios(self.store.root(), self.pane_ratios)
                        {
                            self.set_status(
                                StatusKind::Error,
                                format!("保存分栏宽度失败：{error}"),
                            );
                        }
                    }
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
            Message::ProfileComputerName(v) => self.profile_draft.identity.computer_name = v,
            Message::ProfileUserName(v) => self.profile_draft.identity.user_name = v,
            Message::ProfileMacAddress(v) => self.profile_draft.identity.mac_address = v,
            Message::ProfileMacAddressGenerate => {
                let mut bytes = *Uuid::new_v4().as_bytes();
                bytes[0] = (bytes[0] & 0xfc) | 0x02;
                self.profile_draft.identity.mac_address = bytes[..6]
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(":");
            }
            Message::ProfileMachineGuid(v) => self.profile_draft.identity.machine_guid = v,
            Message::ProfileMachineGuidGenerate => {
                self.profile_draft.identity.machine_guid = Uuid::new_v4().to_string();
            }
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
            Message::ProfileDnsTlsRevocation(value) => {
                self.profile_draft.dns_editor.draft.tls_revocation = value
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

#[cfg(test)]
mod pane_layout_tests {
    use super::*;
    use iced::widget::pane_grid::{Node, ResizeEvent, State};

    fn ratio(panes: &State<bool>) -> f32 {
        match panes.layout() {
            Node::Split { ratio, .. } => *ratio,
            _ => panic!("expected list/detail split"),
        }
    }

    fn resize(app: &mut EnvBoxApp, nav: Nav, value: f32) {
        app.nav = nav;
        let panes = if nav == Nav::Profiles {
            &app.profile_panes
        } else {
            &app.content_panes
        };
        let split = *panes.layout().splits().next().unwrap();
        let _ = app.update(Message::ContentPaneResized(ResizeEvent {
            split,
            ratio: value,
        }));
    }

    #[test]
    fn pane_widths_survive_restart_and_close_preference_changes() {
        let root = std::env::temp_dir().join(format!("aura-pane-test-{}", Uuid::new_v4()));
        close_behavior::save(&root, CloseBehavior::Exit).unwrap();
        let (mut app, _) = EnvBoxApp::new_with_store(ConfigStore::new(&root));
        assert_eq!(ratio(&app.content_panes), 0.60);
        assert_eq!(ratio(&app.profile_panes), 0.60);
        resize(&mut app, Nav::Apps, 0.67);
        resize(&mut app, Nav::Profiles, 0.44);
        assert_eq!(close_behavior::load(&root), CloseBehavior::Exit);
        close_behavior::save(&root, CloseBehavior::Tray).unwrap();
        let (reopened, _) = EnvBoxApp::new_with_store(ConfigStore::new(&root));
        assert_eq!(ratio(&reopened.content_panes), 0.67);
        assert_eq!(ratio(&reopened.profile_panes), 0.44);
        assert_eq!(reopened.close_behavior, CloseBehavior::Tray);
        std::fs::remove_dir_all(root).unwrap();
    }
}
