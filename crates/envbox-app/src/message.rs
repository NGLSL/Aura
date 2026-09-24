//! UI message and form enums. Domain types stay in envbox-core.

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Apps,
    Profiles,
    Instances,
    Audit,
    Settings,
}

impl Nav {
    pub const ALL: [Nav; 5] = [
        Nav::Apps,
        Nav::Profiles,
        Nav::Instances,
        Nav::Audit,
        Nav::Settings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Nav::Apps => "应用",
            Nav::Profiles => "配置文件",
            Nav::Instances => "运行实例",
            Nav::Audit => "审计",
            Nav::Settings => "设置",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsChoice {
    Host,
    VirtualView,
}

impl DnsChoice {
    pub const ALL: [DnsChoice; 2] = [DnsChoice::Host, DnsChoice::VirtualView];

    pub fn to_mode(self) -> envbox_core::DnsMode {
        match self {
            DnsChoice::Host => envbox_core::DnsMode::Host,
            DnsChoice::VirtualView => envbox_core::DnsMode::VirtualView,
        }
    }

    pub fn from_mode(m: &envbox_core::DnsMode) -> Self {
        match m {
            envbox_core::DnsMode::Host => DnsChoice::Host,
            envbox_core::DnsMode::VirtualView => DnsChoice::VirtualView,
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
pub struct NamedId {
    pub name: String,
    pub id: Uuid,
}

impl std::fmt::Display for NamedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKind {
    Command,
    Executable,
}

impl LaunchKind {
    pub const ALL: [LaunchKind; 2] = [LaunchKind::Command, LaunchKind::Executable];
}

impl std::fmt::Display for LaunchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchKind::Command => write!(f, "Command"),
            LaunchKind::Executable => write!(f, "Executable"),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Basic,
    Env,
    Advanced,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Instances,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Success,
    Error,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Message {
    Nav(Nav),
    DetailTab(DetailTab),
    BottomTab(BottomTab),
    Search(String),
    // Application form
    AppNew,
    AppPickerQuery(String),
    AppPickerSelect(usize),
    AppPickerUseSelected,
    AppPickerManual,
    AppPickerClose,
    /// Background discovery finished (items already icon-cached).
    AppPickerLoaded(Vec<crate::discover::DiscoveredApp>),
    /// Picker icon PNGs extracted in parallel (key → png).
    AppPickerIcons(Vec<(String, std::path::PathBuf)>),
    /// Icon PNGs extracted for configured applications.
    AppIconsLoaded(Vec<(Uuid, std::path::PathBuf)>),
    AppName(String),
    AppLaunchKind(LaunchKind),
    AppPath(String),
    AppArgs(String),
    AppWorkDir(String),
    AppProfile(Uuid),
    AppInherit(bool),
    AppAudit(bool),
    AppSelect(Uuid),
    AppSave,
    AppDelete,
    AppRun,
    AppRunId(Uuid),
    AppStopId(Uuid),
    /// nil = Host (real host, no virtualization)
    AppRunWith(Uuid),
    AppOpenLocation,
    AppBrowseWorkDir,
    AppToggleAuditCollapse,
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
    // Instances / audit
    InstanceRefresh,
    InstanceStop(Uuid),
    AuditRefresh,
    OpenAuditDir,
    // Settings / diagnostics
    OpenConfigDir,
    RunProbe,
    // Custom window chrome
    WindowDrag,
    WindowMinimize,
    WindowToggleMaximize,
    WindowClose,
}
