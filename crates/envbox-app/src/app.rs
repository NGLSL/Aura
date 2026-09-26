//! EnvBox GUI application state and message update.
//! View rendering lives in `app/` modules; injection logic never lives here.

use envbox_core::{
    Application, AuditEvent, BrowserPrivacyProfile, DnsProfile, EnvironmentProfile,
    InstanceStatus, LaunchTarget, LocaleProfile, RegistryProfile, TimezoneProfile,
};
use envbox_launcher::{format_args, parse_args, InstanceManager, RunTarget};
use envbox_storage::{
    enumerate_dynamic_timezone_ids, validate_application, validate_profile, windows_id_to_iana,
    ConfigStore,
};
use iced::Task;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

use crate::message::{
    BottomTab, ComboField, DetailTab, DnsChoice, LaunchKind, Message, Nav, StatusKind, WebRtcChoice,
};
use crate::options;

pub struct AppDraft {
    pub id: Option<Uuid>,
    pub name: String,
    pub kind: LaunchKind,
    pub path: String,
    pub args: String,
    pub work_dir: String,
    pub profile_id: Uuid,
    pub inherit: bool,
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
            audit: false,
            icon_src: String::new(),
        }
    }
}

pub struct ProfileDraft {
    pub id: Option<Uuid>,
    pub name: String,
    pub locale: String,
    pub ui: String,
    pub region: String,
    pub tz: String,
    pub tz_iana: String,
    pub dns_mode: DnsChoice,
    pub dns_servers: String,
    pub webrtc: WebRtcChoice,
    pub env: String,
}

pub enum RunSelection {
    Default,
    Profile(Uuid),
    Host,
}

pub struct EnvBoxApp {
    pub store: ConfigStore,
    pub nav: Nav,
    pub detail_tab: DetailTab,
    pub bottom_tab: BottomTab,
    pub search: String,
    pub applications: Vec<Application>,
    pub profiles: Vec<EnvironmentProfile>,
    pub timezones: Vec<String>,
    pub instances: InstanceManager,
    pub app_draft: AppDraft,
    pub profile_draft: ProfileDraft,
    pub audit_events: Vec<AuditEvent>,
    /// Empty = show all software.
    pub audit_filter: String,
    /// Open searchable-select field in the Profile editor.
    pub open_combo: Option<ComboField>,
    pub combo_query: String,
    pub status: String,
    pub status_kind: StatusKind,
    /// Local app picker (Kite-style discovery). `None` = closed.
    pub app_picker: Option<AppPickerState>,
    /// Cached icon PNGs for configured applications (uuid → png).
    pub app_icons: HashMap<Uuid, PathBuf>,
}

/// Local installed-app picker state.
#[derive(Debug, Clone, Default)]
pub struct AppPickerState {
    pub query: String,
    pub items: Vec<crate::discover::DiscoveredApp>,
    pub selected: Option<usize>,
    pub scanned: bool,
    pub loading: bool,
}

impl EnvBoxApp {
    pub fn new() -> (Self, Task<Message>) {
        let store = ConfigStore::new(ConfigStore::default_root());
        let _ = store.ensure_dirs();
        let applications = store
            .load_applications()
            .map(|d| d.applications)
            .unwrap_or_default();
        let profiles = store.load_profiles().map(|d| d.profiles).unwrap_or_default();
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
            dns_servers: String::new(),
            webrtc: WebRtcChoice::Host,
            env: String::new(),
        };
        let mut app = Self {
            store,
            nav: Nav::Apps,
            detail_tab: DetailTab::Basic,
            bottom_tab: BottomTab::Instances,
            search: String::new(),
            applications,
            profiles,
            timezones,
            instances: InstanceManager::new(),
            app_draft,
            profile_draft,
            audit_events: Vec::new(),
            audit_filter: String::new(),
            open_combo: None,
            combo_query: String::new(),
            status: String::new(),
            status_kind: StatusKind::Info,
            app_picker: None,
            app_icons: HashMap::new(),
        };
        app.load_audit();
        let icons_task = app.refresh_app_icons();
        if let Some(first) = app.applications.first() {
            let id = first.id;
            app.select_app(id);
        }
        (app, icons_task)
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
                audit: app.audit,
                icon_src: match &app.launch {
                    LaunchTarget::Command { command } => command.clone(),
                    LaunchTarget::Executable { path } => path.display().to_string(),
                    LaunchTarget::Packaged { aumid, .. } => aumid.clone(),
                },
            };
        }
    }

    pub fn set_status(&mut self, kind: StatusKind, msg: impl Into<String>) {
        self.status_kind = kind;
        self.status = msg.into();
    }

    /// Icon cache directory beside config store.
    pub fn icon_dir(&self) -> PathBuf {
        self.store.root().join("icons")
    }

    /// Scan into the picker off-thread (Kite-inspired). List first, icons later.
    fn scan_apps_for_picker(&mut self) -> Task<Message> {
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
    fn extract_picker_icons(&mut self) -> Task<Message> {
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
    fn apply_picker_selection(&mut self) -> Task<Message> {
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
        self.app_picker = None;
        match cap.injection {
            crate::package::InjectionSupport::Supported => {
                self.set_status(
                    StatusKind::Success,
                    format!("已带入「{}」（{}），请确认 Profile 后保存", item.name, cap.runtime_label),
                );
            }
            crate::package::InjectionSupport::Delayed => {
                self.set_status(
                    StatusKind::Info,
                    format!(
                        "已带入「{}」：{} / {}。{}",
                        item.name, cap.runtime_label, cap.trust_label, cap.reason
                    ),
                );
            }
            crate::package::InjectionSupport::Unsupported => {
                self.set_status(
                    StatusKind::Error,
                    format!(
                        "已带入「{}」，但当前不支持 Runtime 注入：{} / {}。{}",
                        item.name, cap.runtime_label, cap.trust_label, cap.reason
                    ),
                );
            }
        }
        Task::none()
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
        if let Some(img) = ev.image.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
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

    pub fn filtered_apps(&self) -> Vec<&Application> {        let q = self.search.trim().to_ascii_lowercase();
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

    pub fn is_app_running(&self, id: Uuid) -> bool {
        self.instances
            .list()
            .into_iter()
            .any(|i| i.application_id == id && i.status == InstanceStatus::Running)
    }

    pub fn view(&self) -> iced::Element<'_, Message> {
        crate::views::view(self)
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Nav(n) => self.nav = n,
            Message::DetailTab(t) => self.detail_tab = t,
            Message::BottomTab(t) => {
                self.bottom_tab = t;
                if t == BottomTab::Audit {
                    self.load_audit();
                }
            }
            Message::Search(v) => self.search = v,
            Message::AppNew => {
                self.app_draft =
                    AppDraft::blank(self.profiles.first().map(|p| p.id).unwrap_or_default());
                // Open local picker (Kite-style discovery) first.
                self.app_picker = Some(AppPickerState::default());
                return self.scan_apps_for_picker();
            }
            Message::AppPickerQuery(q) => {
                if let Some(p) = self.app_picker.as_mut() {
                    p.query = q;
                    // Keep selection when the same row is still visible.
                    if let Some(sel) = p.selected {
                        let q = p.query.trim().to_ascii_lowercase();
                        let still = p.items.get(sel).map(|a| {
                            q.is_empty()
                                || a.name.to_ascii_lowercase().contains(&q)
                                || a.path.to_ascii_lowercase().contains(&q)
                        });
                        if !still.unwrap_or(false) {
                            p.selected = None;
                        }
                    }
                }
            }
            Message::AppPickerLoaded(items) => {
                let n = items.len();
                if let Some(p) = self.app_picker.as_mut() {
                    p.items = items;
                    p.loading = false;
                    p.scanned = true;
                    p.selected = p.items.first().map(|_| 0);
                }
                self.set_status(StatusKind::Info, format!("已发现 {n} 个本机应用"));
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
            Message::AppAudit(v) => self.app_draft.audit = v,
            Message::AppSelect(id) => self.select_app(id),
            Message::AppOpenLocation => {
                let target = if !self.app_draft.work_dir.trim().is_empty() {
                    self.app_draft.work_dir.trim().to_string()
                } else if !self.app_draft.path.trim().is_empty() {
                    self.app_draft.path.trim().to_string()
                } else {
                    String::new()
                };
                if !target.is_empty() {
                    let _ = std::process::Command::new("explorer").arg(&target).spawn();
                }
            }
            Message::AppBrowseWorkDir => {
                let target = if !self.app_draft.work_dir.trim().is_empty() {
                    self.app_draft.work_dir.trim().to_string()
                } else {
                    ".".to_string()
                };
                let _ = std::process::Command::new("explorer").arg(&target).spawn();
            }
            Message::AppToggleAuditCollapse => {}
            Message::AppSave => return self.save_app(),
            Message::AppDelete => return self.delete_app(),
            Message::AppRun => return self.run_selected(RunSelection::Default),
            Message::AppRunId(id) => {
                self.select_app(id);
                return self.run_selected(RunSelection::Default);
            }
            Message::AppStopId(id) => {
                let inst_id = self.instances.list().into_iter().find(|i| {
                    i.application_id == id && i.status == envbox_core::InstanceStatus::Running
                }).map(|i| i.id);
                if let Some(iid) = inst_id {
                    if let Err(err) = self.instances.stop(iid) {
                        self.set_status(StatusKind::Error, format!("停止失败: {err}"));
                    } else {
                        self.set_status(StatusKind::Success, "已停止运行");
                    }
                }
            }
            Message::AppRunWith(id) => {
                let sel = if id.is_nil() {
                    RunSelection::Host
                } else {
                    RunSelection::Profile(id)
                };
                return self.run_selected(sel);
            }
            Message::ProfileName(v) => self.profile_draft.name = v,
            Message::ProfileLocale(v) => self.profile_draft.locale = v,
            Message::ProfileUi(v) => self.profile_draft.ui = v,
            Message::ProfileRegion(v) => self.profile_draft.region = v,
            Message::ProfileTz(v) => {
                self.profile_draft.tz_iana =
                    windows_id_to_iana(&v).unwrap_or_default().to_string();
                self.profile_draft.tz = v;
            }
            Message::ProfileTzIana(v) => self.profile_draft.tz_iana = v,
            Message::ProfileDnsMode(m) => self.profile_draft.dns_mode = m,
            Message::ProfileDnsServers(v) => self.profile_draft.dns_servers = v,
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
                if let Some(p) = self.profiles.iter().find(|p| p.id == id) {
                    self.profile_draft = profile_to_draft(p);
                }
                self.open_combo = None;
                self.combo_query.clear();
            }
            Message::ProfileSave => return self.save_profile(),
            Message::ProfileDelete => return self.delete_profile(),
            Message::ProfileNew => {
                let tz = self
                    .timezones
                    .iter()
                    .find(|t| t.eq_ignore_ascii_case("Pacific Standard Time"))
                    .cloned()
                    .or_else(|| self.timezones.first().cloned())
                    .unwrap_or_default();
                let tz_iana = windows_id_to_iana(&tz).unwrap_or_default().to_string();
                self.profile_draft = ProfileDraft {
                    id: None,
                    name: String::new(),
                    locale: "en-US".into(),
                    ui: "en-US".into(),
                    region: "US".into(),
                    tz,
                    tz_iana,
                    dns_mode: DnsChoice::Host,
                    dns_servers: String::new(),
                    webrtc: WebRtcChoice::Host,
                    env: String::new(),
                };
                self.open_combo = None;
                self.combo_query.clear();
            }
            Message::InstanceRefresh => {
                self.instances.refresh_all();
                self.set_status(StatusKind::Success, "实例已刷新");
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
                        .args(["/C", "start", "Aura · EnvBox Probe", "cmd.exe", "/K", &probe_str])
                        .spawn();
                    if spawn_res.is_ok() {
                        self.set_status(StatusKind::Success, "已在独立控制台中运行环境探针");
                    } else {
                        self.set_status(StatusKind::Error, "运行环境探针启动失败");
                    }
                } else {
                    self.set_status(StatusKind::Error, "未找到 envbox-probe.exe，请先通过 cargo build 构建探针");
                }
            }
            Message::WindowDrag => {
                return iced::window::get_latest().then(|id| {
                    id.map(iced::window::drag).unwrap_or_else(Task::none)
                })
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
            Message::WindowClose => {
                return iced::window::get_latest().then(|id| {
                    id.map(iced::window::close).unwrap_or_else(Task::none)
                })
            }
        }
        Task::none()
    }

    fn save_app(&mut self) -> Task<Message> {
        if self.profiles.is_empty() {
            self.set_status(StatusKind::Error, "保存失败：请先创建配置文件");
            return Task::none();
        }
        if !self
            .profiles
            .iter()
            .any(|p| p.id == self.app_draft.profile_id)
        {
            self.set_status(StatusKind::Error, "保存失败：默认配置文件不存在");
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
                self.app_draft.id = None;
                self.set_status(StatusKind::Success, "应用已删除");
            }
            Err(err) => self.set_status(StatusKind::Error, format!("删除失败: {err}")),
        }
        Task::none()
    }

    fn run_selected(&mut self, sel: RunSelection) -> Task<Message> {
        let Some(app_id) = self.app_draft.id else {
            self.set_status(StatusKind::Error, "请先选择应用");
            return Task::none();
        };
        let Some(app) = self.applications.iter().find(|a| a.id == app_id).cloned() else {
            self.set_status(StatusKind::Error, "应用不存在");
            return Task::none();
        };
        let target = match sel {
            RunSelection::Host => RunTarget::Host,
            RunSelection::Default => {
                let Some(profile) = self
                    .profiles
                    .iter()
                    .find(|p| p.id == app.default_profile_id)
                    .cloned()
                else {
                    self.set_status(StatusKind::Error, "配置文件不存在");
                    return Task::none();
                };
                RunTarget::Profile(profile)
            }
            RunSelection::Profile(id) => {
                let Some(profile) = self.profiles.iter().find(|p| p.id == id).cloned() else {
                    self.set_status(StatusKind::Error, "配置文件不存在");
                    return Task::none();
                };
                RunTarget::Profile(profile)
            }
        };
        match self.instances.run(&app, target) {
            Ok(id) => {
                self.set_status(StatusKind::Success, format!("已启动实例 {id}"));
                self.nav = Nav::Apps;
                self.bottom_tab = BottomTab::Instances;
                self.instances.refresh_all();
            }
            Err(err) => self.set_status(StatusKind::Error, format!("启动失败: {err}")),
        }
        Task::none()
    }

    fn save_profile(&mut self) -> Task<Message> {
        let mut servers = Vec::new();
        for tok in self
            .profile_draft
            .dns_servers
            .split(|c| c == ',' || c == ' ' || c == ';')
            .filter(|s| !s.is_empty())
        {
            match tok.parse() {
                Ok(ip) => servers.push(ip),
                Err(_) => {
                    self.set_status(StatusKind::Error, format!("保存失败：无效 DNS 服务器 {tok:?}"));
                    return Task::none();
                }
            }
        }
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
            dns: DnsProfile {
                mode: self.profile_draft.dns_mode.to_mode(),
                servers,
            },
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
        let mut doc = self.store.load_profiles().unwrap_or_default();
        if let Some(slot) = doc.profiles.iter_mut().find(|p| p.id == profile.id) {
            *slot = profile.clone();
        } else {
            doc.profiles.push(profile.clone());
        }
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                self.profile_draft.id = Some(profile.id);
                self.set_status(StatusKind::Success, "配置文件已保存");
            }
            Err(err) => self.set_status(StatusKind::Error, format!("保存失败: {err}")),
        }
        Task::none()
    }

    fn delete_profile(&mut self) -> Task<Message> {
        let Some(id) = self.profile_draft.id else {
            self.set_status(StatusKind::Error, "请先选择配置文件");
            return Task::none();
        };
        let mut doc = self.store.load_profiles().unwrap_or_default();
        doc.profiles.retain(|p| p.id != id);
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                self.profile_draft.id = None;
                self.set_status(StatusKind::Success, "配置文件已删除");
            }
            Err(err) => self.set_status(StatusKind::Error, format!("删除失败: {err}")),
        }
        Task::none()
    }
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
        dns_servers: p
            .dns
            .servers
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(","),
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
