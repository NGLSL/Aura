//! EnvBox GUI (tickets 10-11). Thin shell over envbox-core / storage / launcher.
//! Injection logic never lives here.

use envbox_core::{
    Application, DnsMode, DnsProfile, EnvironmentProfile, InstanceStatus, LaunchTarget,
    LocaleProfile, RegistryProfile, TimezoneProfile,
};
use envbox_launcher::{format_args, parse_args, InstanceManager, RunTarget};
use envbox_storage::{
    enumerate_dynamic_timezone_ids, validate_application, validate_profile, windows_id_to_iana,
    ConfigStore,
};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input, Column,
};
use iced::{Element, Fill, Task, Theme};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

fn main() -> iced::Result {
    iced::application("EnvBox", EnvBoxApp::update, EnvBoxApp::view)
        .theme(|_| Theme::Light)
        .run_with(EnvBoxApp::new)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Applications,
    Profiles,
    Instances,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsChoice {
    Host,
    VirtualView,
}

impl DnsChoice {
    const ALL: [DnsChoice; 2] = [DnsChoice::Host, DnsChoice::VirtualView];
    fn to_mode(self) -> DnsMode {
        match self {
            DnsChoice::Host => DnsMode::Host,
            DnsChoice::VirtualView => DnsMode::VirtualView,
        }
    }
    fn from_mode(m: &DnsMode) -> Self {
        match m {
            DnsMode::Host => DnsChoice::Host,
            DnsMode::VirtualView => DnsChoice::VirtualView,
        }
    }
}

impl std::fmt::Display for DnsChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DnsChoice::Host => write!(f, "Host"),
            DnsChoice::VirtualView => write!(f, "VirtualView"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NamedId {
    name: String,
    id: Uuid,
}

impl std::fmt::Display for NamedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[derive(Debug, Clone)]
enum Message {
    Tab(Tab),
    // Application form
    AppNew,
    AppName(String),
    AppLaunchKind(LaunchKind),
    AppPath(String),
    AppArgs(String),
    AppWorkDir(String),
    AppProfile(Uuid),
    AppInherit(bool),
    AppSelect(Uuid),
    AppSave,
    AppDelete,
    AppRun,
    AppRunWith(Uuid), // nil = Host (real host, no virtualization)
    // Profile form
    ProfileName(String),
    ProfileLocale(String),
    ProfileUi(String),
    ProfileRegion(String),
    ProfileTz(String),
    ProfileTzIana(String),
    ProfileDnsMode(DnsChoice),
    ProfileDnsServers(String),
    ProfileEnv(String),
    ProfileSelect(Uuid),
    ProfileSave,
    ProfileDelete,
    ProfileNew,
    // Instances
    InstanceRefresh,
    InstanceStop(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchKind {
    Command,
    Executable,
}

impl LaunchKind {
    const ALL: [LaunchKind; 2] = [LaunchKind::Command, LaunchKind::Executable];
}

impl std::fmt::Display for LaunchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchKind::Command => write!(f, "Command"),
            LaunchKind::Executable => write!(f, "Executable"),
        }
    }
}

struct AppDraft {
    id: Option<Uuid>,
    name: String,
    kind: LaunchKind,
    path: String,
    args: String,
    work_dir: String,
    profile_id: Uuid,
    inherit: bool,
}

impl AppDraft {
    fn blank(profile_id: Uuid) -> Self {
        Self {
            id: None,
            name: String::new(),
            kind: LaunchKind::Command,
            path: String::new(),
            args: String::new(),
            work_dir: String::new(),
            profile_id,
            inherit: true,
        }
    }
}

struct ProfileDraft {
    id: Option<Uuid>,
    name: String,
    locale: String,
    ui: String,
    region: String,
    tz: String,
    tz_iana: String,
    dns_mode: DnsChoice,
    dns_servers: String,
    env: String,
}

struct EnvBoxApp {
    store: ConfigStore,
    tab: Tab,
    applications: Vec<Application>,
    profiles: Vec<EnvironmentProfile>,
    timezones: Vec<String>,
    instances: InstanceManager,
    app_draft: AppDraft,
    profile_draft: ProfileDraft,
    status: String,
}

impl EnvBoxApp {
    fn new() -> (Self, Task<Message>) {
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
            dns_servers: String::new(),
            env: String::new(),
        };
        (
            Self {
                store,
                tab: Tab::Applications,
                applications,
                profiles,
                timezones,
                instances: InstanceManager::new(),
                app_draft,
                profile_draft,
                status: String::new(),
            },
            Task::none(),
        )
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Tab(t) => self.tab = t,
            Message::AppNew => {
                self.app_draft = AppDraft::blank(
                    self.profiles.first().map(|p| p.id).unwrap_or_default(),
                );
            }
            Message::AppName(v) => self.app_draft.name = v,
            Message::AppLaunchKind(k) => self.app_draft.kind = k,
            Message::AppPath(v) => self.app_draft.path = v,
            Message::AppArgs(v) => self.app_draft.args = v,
            Message::AppWorkDir(v) => self.app_draft.work_dir = v,
            Message::AppProfile(id) => self.app_draft.profile_id = id,
            Message::AppInherit(v) => self.app_draft.inherit = v,
            Message::AppSelect(id) => {
                if let Some(app) = self.applications.iter().find(|a| a.id == id) {
                    self.app_draft = AppDraft {
                        id: Some(app.id),
                        name: app.name.clone(),
                        kind: match &app.launch {
                            LaunchTarget::Command { .. } => LaunchKind::Command,
                            LaunchTarget::Executable { .. } => LaunchKind::Executable,
                        },
                        path: match &app.launch {
                            LaunchTarget::Command { command } => command.clone(),
                            LaunchTarget::Executable { path } => path.display().to_string(),
                        },
                        args: format_args(&app.arguments),
                        work_dir: app
                            .working_directory
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        profile_id: app.default_profile_id,
                        inherit: app.inherit_children,
                    };
                }
            }
            Message::AppSave => return self.save_app(),
            Message::AppDelete => return self.delete_app(),
            Message::AppRun => return self.run_selected(RunSelection::Default),
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
                // Always overwrite: unmapped Windows IDs must not keep a stale IANA.
                self.profile_draft.tz_iana =
                    windows_id_to_iana(&v).unwrap_or_default().to_string();
                self.profile_draft.tz = v;
            }
            Message::ProfileTzIana(v) => self.profile_draft.tz_iana = v,
            Message::ProfileDnsMode(m) => self.profile_draft.dns_mode = m,
            Message::ProfileDnsServers(v) => self.profile_draft.dns_servers = v,
            Message::ProfileEnv(v) => self.profile_draft.env = v,
            Message::ProfileSelect(id) => {
                if let Some(p) = self.profiles.iter().find(|p| p.id == id) {
                    self.profile_draft = profile_to_draft(p);
                }
            }
            Message::ProfileSave => return self.save_profile(),
            Message::ProfileDelete => return self.delete_profile(),
            Message::ProfileNew => {
                let tz = self.timezones.first().cloned().unwrap_or_default();
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
                    env: String::new(),
                };
            }
            Message::InstanceRefresh => {
                self.instances.refresh_all();
            }
            Message::InstanceStop(id) => {
                if let Err(err) = self.instances.stop(id) {
                    self.status = format!("Stop failed: {err}");
                } else {
                    self.status = "Stopped".into();
                }
            }
        }
        Task::none()
    }

    fn save_app(&mut self) -> Task<Message> {
        if self.profiles.is_empty() {
            self.status = "Save failed: create a Profile first".into();
            return Task::none();
        }
        if !self
            .profiles
            .iter()
            .any(|p| p.id == self.app_draft.profile_id)
        {
            self.status = "Save failed: default Profile not found".into();
            return Task::none();
        }
        let launch = match self.app_draft.kind {
            LaunchKind::Command => LaunchTarget::Command {
                command: self.app_draft.path.clone(),
            },
            LaunchKind::Executable => LaunchTarget::Executable {
                path: PathBuf::from(self.app_draft.path.clone()),
            },
        };
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
        };
        if let Err(err) = validate_application(&app) {
            self.status = format!("Save failed: {err}");
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
                self.status = "Application saved".into();
            }
            Err(err) => self.status = format!("Save failed: {err}"),
        }
        Task::none()
    }

    fn delete_app(&mut self) -> Task<Message> {
        let Some(id) = self.app_draft.id else {
            self.status = "Select an Application first".into();
            return Task::none();
        };
        let mut doc = self.store.load_applications().unwrap_or_default();
        doc.applications.retain(|a| a.id != id);
        match self.store.save_applications(&doc) {
            Ok(()) => {
                self.applications = doc.applications;
                self.app_draft.id = None;
                self.status = "Application deleted".into();
            }
            Err(err) => self.status = format!("Delete failed: {err}"),
        }
        Task::none()
    }

    fn run_selected(&mut self, sel: RunSelection) -> Task<Message> {
        let Some(app_id) = self.app_draft.id else {
            self.status = "Select an Application first".into();
            return Task::none();
        };
        let Some(app) = self.applications.iter().find(|a| a.id == app_id).cloned() else {
            self.status = "Application not found".into();
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
                    self.status = "Profile not found".into();
                    return Task::none();
                };
                RunTarget::Profile(profile)
            }
            RunSelection::Profile(id) => {
                let Some(profile) = self.profiles.iter().find(|p| p.id == id).cloned() else {
                    self.status = "Profile not found".into();
                    return Task::none();
                };
                RunTarget::Profile(profile)
            }
        };
        match self.instances.run(&app, target) {
            Ok(id) => {
                self.status = format!("Started instance {id}");
                self.tab = Tab::Instances;
                self.instances.refresh_all();
            }
            Err(err) => self.status = format!("Run failed: {err}"),
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
                    self.status = format!("Save failed: invalid DNS server {tok:?}");
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
                self.status = format!("Save failed: Env expects KEY=VALUE, got {line:?}");
                return Task::none();
            };
            let k = k.trim();
            if k.is_empty() {
                self.status = format!("Save failed: Env key empty in {line:?}");
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
        };
        if let Err(err) = validate_profile(&profile) {
            self.status = format!("Save failed: {err}");
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
                self.status = "Profile saved".into();
            }
            Err(err) => self.status = format!("Save failed: {err}"),
        }
        Task::none()
    }

    fn delete_profile(&mut self) -> Task<Message> {
        let Some(id) = self.profile_draft.id else {
            self.status = "Select a Profile first".into();
            return Task::none();
        };
        let mut doc = self.store.load_profiles().unwrap_or_default();
        doc.profiles.retain(|p| p.id != id);
        match self.store.save_profiles(&doc) {
            Ok(()) => {
                self.profiles = doc.profiles;
                self.profile_draft.id = None;
                self.status = "Profile deleted".into();
            }
            Err(err) => self.status = format!("Delete failed: {err}"),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let tabs = row![
            tab_btn("Applications", Tab::Applications, self.tab),
            tab_btn("Profiles", Tab::Profiles, self.tab),
            tab_btn("Instances", Tab::Instances, self.tab),
        ]
        .spacing(8);

        let body: Element<_> = match self.tab {
            Tab::Applications => self.view_applications(),
            Tab::Profiles => self.view_profiles(),
            Tab::Instances => self.view_instances(),
        };

        container(
            column![tabs, body, text(&self.status).size(14)]
                .spacing(12)
                .padding(16)
                .width(Fill)
                .height(Fill),
        )
        .into()
    }

    fn view_applications(&self) -> Element<'_, Message> {
        let list: Element<_> = self
            .applications
            .iter()
            .fold(
                Column::new()
                    .spacing(4)
                    .push(button(text("New")).on_press(Message::AppNew)),
                |col, app| {
                    col.push(
                        button(text(format!("{}  [{}]", app.name, app.id)))
                            .on_press(Message::AppSelect(app.id)),
                    )
                },
            )
            .into();

        let profile_labels: Vec<NamedId> = self
            .profiles
            .iter()
            .map(|p| NamedId {
                name: p.name.clone(),
                id: p.id,
            })
            .collect();
        let selected = profile_labels
            .iter()
            .find(|n| n.id == self.app_draft.profile_id)
            .cloned();

        let run_with: Vec<NamedId> = {
            let mut v = vec![NamedId {
                name: "Host".into(),
                id: Uuid::nil(),
            }];
            v.extend(profile_labels.iter().cloned());
            v
        };

        let form = column![
            text("Application").size(20),
            text_input("Name", &self.app_draft.name).on_input(Message::AppName),
            pick_list(
                LaunchKind::ALL,
                Some(self.app_draft.kind),
                Message::AppLaunchKind
            ),
            text_input("Path / Command", &self.app_draft.path).on_input(Message::AppPath),
            text_input("Arguments (quoted)", &self.app_draft.args).on_input(Message::AppArgs),
            text_input("Working Directory", &self.app_draft.work_dir).on_input(Message::AppWorkDir),
            pick_list(profile_labels, selected, |n: NamedId| Message::AppProfile(n.id)),
            checkbox("Child processes inherit profile", self.app_draft.inherit)
                .on_toggle(Message::AppInherit),
            row![
                button(text("Save")).on_press(Message::AppSave),
                button(text("Delete")).on_press(Message::AppDelete),
                button(text("Run")).on_press(Message::AppRun),
                pick_list(run_with, None::<NamedId>, |n: NamedId| Message::AppRunWith(n.id))
                    .placeholder("Run With…"),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .width(Fill);

        row![
            scrollable(list).width(280),
            scrollable(form).width(Fill)
        ]
        .spacing(12)
        .into()
    }

    fn view_profiles(&self) -> Element<'_, Message> {
        let list: Element<_> = self
            .profiles
            .iter()
            .fold(
                Column::new().spacing(4).push(button(text("New")).on_press(Message::ProfileNew)),
                |col, p| {
                    col.push(button(text(p.name.clone())).on_press(Message::ProfileSelect(p.id)))
                },
            )
            .into();

        let form = column![
            text("Environment Profile").size(20),
            text_input("Name", &self.profile_draft.name).on_input(Message::ProfileName),
            text_input("Locale", &self.profile_draft.locale).on_input(Message::ProfileLocale),
            text_input("UI Language", &self.profile_draft.ui).on_input(Message::ProfileUi),
            text_input("Region (ISO-2)", &self.profile_draft.region).on_input(Message::ProfileRegion),
            pick_list(
                self.timezones.clone(),
                Some(self.profile_draft.tz.clone()),
                Message::ProfileTz
            ),
            text_input("IANA Timezone", &self.profile_draft.tz_iana)
                .on_input(Message::ProfileTzIana),
            pick_list(
                DnsChoice::ALL,
                Some(self.profile_draft.dns_mode),
                Message::ProfileDnsMode
            ),
            text_input("DNS servers (comma)", &self.profile_draft.dns_servers)
                .on_input(Message::ProfileDnsServers),
            text_input("Env KEY=VALUE;KEY2=VALUE2", &self.profile_draft.env)
                .on_input(Message::ProfileEnv),
            row![
                button(text("Save")).on_press(Message::ProfileSave),
                button(text("Delete")).on_press(Message::ProfileDelete),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .width(Fill);

        row![
            scrollable(list).width(280),
            scrollable(form).width(Fill)
        ]
        .spacing(12)
        .into()
    }

    fn view_instances(&self) -> Element<'_, Message> {
        let rows = self.instances.list().into_iter().fold(
            Column::new().spacing(6).push(
                button(text("Refresh")).on_press(Message::InstanceRefresh),
            ),
            |col, inst| {
                let count = self.instances.child_count(inst.id).unwrap_or(0);
                let line = format!(
                    "{}  {}  root={}  children={}  profile={}",
                    inst.id,
                    status_label(inst.status),
                    inst.root_pid,
                    count,
                    inst.profile_id
                );
                col.push(
                    row![
                        text(line).size(14),
                        button(text("Stop")).on_press(Message::InstanceStop(inst.id)),
                    ]
                    .spacing(8),
                )
            },
        );
        scrollable(rows).width(Fill).into()
    }
}

enum RunSelection {
    Default,
    Profile(Uuid),
    Host,
}

fn tab_btn(label: &str, tab: Tab, current: Tab) -> Element<'static, Message> {
    let btn = button(text(label.to_string()));
    if tab == current {
        btn.into()
    } else {
        btn.on_press(Message::Tab(tab)).into()
    }
}

fn profile_to_draft(p: &EnvironmentProfile) -> ProfileDraft {
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
        env: p
            .environment
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(";"),
    }
}

fn status_label(s: InstanceStatus) -> &'static str {
    match s {
        InstanceStatus::Starting => "Starting",
        InstanceStatus::Running => "Running",
        InstanceStatus::Stopping => "Stopping",
        InstanceStatus::Exited => "Exited",
        InstanceStatus::Failed => "Failed",
    }
}
