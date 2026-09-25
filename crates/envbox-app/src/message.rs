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

/// WebRTC Privacy form choice (mirrors `DnsChoice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebRtcChoice {
    Host,
    PublicInterfaceOnly,
    ProxyOnly,
    Strict,
}

impl WebRtcChoice {
    pub const ALL: [WebRtcChoice; 4] = [
        WebRtcChoice::Host,
        WebRtcChoice::PublicInterfaceOnly,
        WebRtcChoice::ProxyOnly,
        WebRtcChoice::Strict,
    ];

    pub fn to_policy(self) -> envbox_core::WebRtcPolicy {
        match self {
            WebRtcChoice::Host => envbox_core::WebRtcPolicy::Host,
            WebRtcChoice::PublicInterfaceOnly => envbox_core::WebRtcPolicy::PublicInterfaceOnly,
            WebRtcChoice::ProxyOnly => envbox_core::WebRtcPolicy::ProxyOnly,
            WebRtcChoice::Strict => envbox_core::WebRtcPolicy::Strict,
        }
    }

    pub fn from_policy(p: &envbox_core::WebRtcPolicy) -> Self {
        match p {
            envbox_core::WebRtcPolicy::Host => WebRtcChoice::Host,
            envbox_core::WebRtcPolicy::PublicInterfaceOnly => WebRtcChoice::PublicInterfaceOnly,
            envbox_core::WebRtcPolicy::ProxyOnly => WebRtcChoice::ProxyOnly,
            envbox_core::WebRtcPolicy::Strict => WebRtcChoice::Strict,
        }
    }

    /// Compact badge text for profile cards / instance rows.
    pub fn short_label(self) -> &'static str {
        match self {
            WebRtcChoice::Host => "WebRTC: 宿主",
            WebRtcChoice::PublicInterfaceOnly => "WebRTC: 仅公网",
            WebRtcChoice::ProxyOnly => "WebRTC: 仅代理",
            WebRtcChoice::Strict => "WebRTC: 严格",
        }
    }
}

impl std::fmt::Display for WebRtcChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebRtcChoice::Host => write!(f, "Host"),
            WebRtcChoice::PublicInterfaceOnly => write!(f, "PublicInterfaceOnly"),
            WebRtcChoice::ProxyOnly => write!(f, "ProxyOnly"),
            WebRtcChoice::Strict => write!(f, "Strict"),
        }
    }
}

/// Display-only Browser Guarantee label.
/// `NetworkEnforced` is Strict + Runtime Network Guard (session UDP deny).
pub fn browser_guarantee_label(policy: envbox_core::WebRtcPolicy) -> &'static str {
    match policy.browser_guarantee() {
        envbox_core::BrowserGuarantee::PolicyOnly => "PolicyOnly",
        envbox_core::BrowserGuarantee::NetworkEnforced => "NetworkEnforced",
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
    ProfileWebRtc(WebRtcChoice),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webrtc_choice_round_trips() {
        for c in WebRtcChoice::ALL {
            assert_eq!(WebRtcChoice::from_policy(&c.to_policy()), c);
        }
        assert_eq!(
            WebRtcChoice::from_policy(&envbox_core::WebRtcPolicy::Host),
            WebRtcChoice::Host
        );
    }

    #[test]
    fn webrtc_guarantee_labels_match_domain() {
        assert_eq!(
            browser_guarantee_label(envbox_core::WebRtcPolicy::ProxyOnly),
            "PolicyOnly"
        );
        assert!(browser_guarantee_label(envbox_core::WebRtcPolicy::Strict).contains("NetworkEnforced"));
    }
}
