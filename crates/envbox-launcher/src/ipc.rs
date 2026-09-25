//! Runtime ↔ Host IPC protocol (packaged-v1 ticket 40).
//!
//! Wire format matches `runtime/src/ipc_bootstrap.h`:
//! one line per message, UTF-8, `MSG_NAME key=value ...`, values are bare
//! tokens or double-quoted strings (`\\` `\"` `\n` `\r` `\t` escapes).
//! Lists use repeated keys (`dns_server=`, `registry_path=`).
//!
//! Public message names (stable, case-sensitive):
//! HELLO / GET_PROFILE / PROFILE / RUNTIME_READY / HOOK_ERROR /
//! PROCESS_CREATED / PROCESS_EXITED.
//!
//! Win32 keeps ENVBOX_* structured values as fallback; IPC/Broker is preferred
//! for all roots (including packaged, which have no Environment Block).

use envbox_core::{DnsMode, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

/// Default pipe name. Override with `ENVBOX_IPC_PIPE`.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\envbox-runtime";

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("pipe connect failed: {0}")]
    Connect(String),
    #[error("pipe io failed: {0}")]
    Io(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("timeout waiting for {0}")]
    Timeout(String),
}

/// One protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcMessage {
    Hello {
        pid: u32,
        instance_id: String,
    },
    GetProfile {
        pid: u32,
        profile_id: String,
    },
    Profile {
        profile_id: String,
        instance_id: String,
        locale_name: String,
        ui_language: String,
        region: String,
        tz_windows: String,
        tz_iana: String,
        inherit_children: bool,
        audit: bool,
        dns_mode: bool,
        dns_servers: Vec<String>,
        registry_paths: Vec<String>,
        /// Browser / Network Guard WebRTC policy (`host`/`public_interface_only`/`proxy_only`/`strict`).
        /// C++ stores the token; business semantics stay in envbox-core.
        webrtc: String,
    },
    RuntimeReady {
        pid: u32,
    },
    HookError {
        pid: u32,
        api: String,
        code: u32,
        detail: String,
    },
    ProcessCreated {
        pid: u32,
        child_pid: u32,
        image: String,
    },
    ProcessExited {
        pid: u32,
        exit_code: u32,
    },
    /// Host → Broker: register a Profile payload in the Session Registry.
    RegisterProfile {
        profile_id: String,
        instance_id: String,
        locale_name: String,
        ui_language: String,
        region: String,
        tz_windows: String,
        tz_iana: String,
        inherit_children: bool,
        audit: bool,
        dns_mode: bool,
        dns_servers: Vec<String>,
        registry_paths: Vec<String>,
        /// See `IpcMessage::Profile::webrtc`.
        webrtc: String,
    },
    /// Host → Broker: bind a PID to a session profile (and optional parent).
    BindPid {
        pid: u32,
        profile_id: String,
        parent_pid: u32,
    },
    /// Unknown / future message (forward compatible).
    Other {
        name: String,
        fields: Vec<(String, String)>,
    },
}

impl IpcMessage {
    pub fn name(&self) -> &'static str {
        match self {
            IpcMessage::Hello { .. } => "HELLO",
            IpcMessage::GetProfile { .. } => "GET_PROFILE",
            IpcMessage::Profile { .. } => "PROFILE",
            IpcMessage::RuntimeReady { .. } => "RUNTIME_READY",
            IpcMessage::HookError { .. } => "HOOK_ERROR",
            IpcMessage::ProcessCreated { .. } => "PROCESS_CREATED",
            IpcMessage::ProcessExited { .. } => "PROCESS_EXITED",
            IpcMessage::RegisterProfile { .. } => "REGISTER_PROFILE",
            IpcMessage::BindPid { .. } => "BIND_PID",
            IpcMessage::Other { .. } => "OTHER",
        }
    }

    /// Encode as one line (no trailing newline).
    pub fn encode_line(&self) -> String {
        match self {
            IpcMessage::Hello { pid, instance_id } => format!(
                "HELLO pid={pid} instance_id={}",
                quote_value(instance_id)
            ),
            IpcMessage::GetProfile { pid, profile_id } => format!(
                "GET_PROFILE pid={pid} profile_id={}",
                quote_value(profile_id)
            ),
            IpcMessage::Profile {
                profile_id,
                instance_id,
                locale_name,
                ui_language,
                region,
                tz_windows,
                tz_iana,
                inherit_children,
                audit,
                dns_mode,
                dns_servers,
                registry_paths,
                webrtc,
            } => {
                let mut s = format!(
                    "PROFILE profile_id={} instance_id={} locale_name={} ui_language={} region={} tz_windows={} tz_iana={} inherit_children={} audit={} dns_mode={} webrtc={}",
                    quote_value(profile_id),
                    quote_value(instance_id),
                    quote_value(locale_name),
                    quote_value(ui_language),
                    quote_value(region),
                    quote_value(tz_windows),
                    quote_value(tz_iana),
                    if *inherit_children { 1 } else { 0 },
                    if *audit { 1 } else { 0 },
                    if *dns_mode { 1 } else { 0 },
                    quote_value(webrtc),
                );
                for d in dns_servers {
                    s.push_str(&format!(" dns_server={}", quote_value(d)));
                }
                for p in registry_paths {
                    s.push_str(&format!(" registry_path={}", quote_value(p)));
                }
                s
            }
            IpcMessage::RuntimeReady { pid } => format!("RUNTIME_READY pid={pid}"),
            IpcMessage::HookError {
                pid,
                api,
                code,
                detail,
            } => {
                let mut s = format!(
                    "HOOK_ERROR pid={pid} api={} code={code}",
                    quote_value(api)
                );
                if !detail.is_empty() {
                    s.push_str(&format!(" detail={}", quote_value(detail)));
                }
                s
            }
            IpcMessage::ProcessCreated {
                pid,
                child_pid,
                image,
            } => {
                let mut s = format!("PROCESS_CREATED pid={pid} child_pid={child_pid}");
                if !image.is_empty() {
                    s.push_str(&format!(" image={}", quote_value(image)));
                }
                s
            }
            IpcMessage::ProcessExited { pid, exit_code } => {
                format!("PROCESS_EXITED pid={pid} exit_code={exit_code}")
            }
            IpcMessage::RegisterProfile {
                profile_id,
                instance_id,
                locale_name,
                ui_language,
                region,
                tz_windows,
                tz_iana,
                inherit_children,
                audit,
                dns_mode,
                dns_servers,
                registry_paths,
                webrtc,
            } => {
                let mut s = format!(
                    "REGISTER_PROFILE profile_id={} instance_id={} locale_name={} ui_language={} region={} tz_windows={} tz_iana={} inherit_children={} audit={} dns_mode={} webrtc={}",
                    quote_value(profile_id),
                    quote_value(instance_id),
                    quote_value(locale_name),
                    quote_value(ui_language),
                    quote_value(region),
                    quote_value(tz_windows),
                    quote_value(tz_iana),
                    if *inherit_children { 1 } else { 0 },
                    if *audit { 1 } else { 0 },
                    if *dns_mode { 1 } else { 0 },
                    quote_value(webrtc),
                );
                for d in dns_servers {
                    s.push_str(&format!(" dns_server={}", quote_value(d)));
                }
                for p in registry_paths {
                    s.push_str(&format!(" registry_path={}", quote_value(p)));
                }
                s
            }
            IpcMessage::BindPid {
                pid,
                profile_id,
                parent_pid,
            } => {
                format!(
                    "BIND_PID pid={pid} profile_id={} parent_pid={parent_pid}",
                    quote_value(profile_id)
                )
            }
            IpcMessage::Other { name, fields } => {
                let mut s = name.clone();
                for (k, v) in fields {
                    s.push(' ');
                    s.push_str(k);
                    s.push('=');
                    s.push_str(&quote_value(v));
                }
                s
            }
        }
    }

    /// Decode one line.
    pub fn decode_line(line: &str) -> Result<Self, IpcError> {
        let line = line.trim_end_matches(['\r', '\n']).trim();
        if line.is_empty() {
            return Err(IpcError::Protocol("empty message".into()));
        }
        let tokens = tokenize_line(line);
        if tokens.is_empty() {
            return Err(IpcError::Protocol("empty message".into()));
        }
        let name = tokens[0].clone();
        let mut map: Vec<(String, String)> = Vec::new();
        let mut lists: HashMap<String, Vec<String>> = HashMap::new();
        for tok in &tokens[1..] {
            if let Some((k, v)) = tok.split_once('=') {
                lists.entry(k.to_string()).or_default().push(v.to_string());
                map.push((k.to_string(), v.to_string()));
            }
        }
        let first = |k: &str| {
            map.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let get_u32 = |k: &str| {
            first(k)
                .parse::<u32>()
                .map_err(|_| IpcError::Protocol(format!("missing/invalid {k}")))
        };
        let get_flag = |k: &str| first(k) == "1";
        let list = |k: &str| lists.get(k).cloned().unwrap_or_default();

        Ok(match name.as_str() {
            "HELLO" => IpcMessage::Hello {
                pid: get_u32("pid")?,
                instance_id: first("instance_id"),
            },
            "GET_PROFILE" => IpcMessage::GetProfile {
                pid: get_u32("pid")?,
                profile_id: first("profile_id"),
            },
            "PROFILE" => IpcMessage::Profile {
                profile_id: first("profile_id"),
                instance_id: first("instance_id"),
                locale_name: first("locale_name"),
                ui_language: first("ui_language"),
                region: first("region"),
                tz_windows: first("tz_windows"),
                tz_iana: first("tz_iana"),
                inherit_children: get_flag("inherit_children"),
                audit: get_flag("audit"),
                dns_mode: get_flag("dns_mode"),
                dns_servers: list("dns_server"),
                registry_paths: list("registry_path"),
                webrtc: first("webrtc"),
            },
            "RUNTIME_READY" => IpcMessage::RuntimeReady {
                pid: get_u32("pid")?,
            },
            "HOOK_ERROR" => IpcMessage::HookError {
                pid: get_u32("pid")?,
                api: first("api"),
                code: first("code").parse().unwrap_or(0),
                detail: first("detail"),
            },
            "PROCESS_CREATED" => IpcMessage::ProcessCreated {
                pid: get_u32("pid")?,
                child_pid: get_u32("child_pid")?,
                image: first("image"),
            },
            "PROCESS_EXITED" => IpcMessage::ProcessExited {
                pid: get_u32("pid")?,
                exit_code: first("exit_code").parse().unwrap_or(0),
            },
            "REGISTER_PROFILE" => IpcMessage::RegisterProfile {
                profile_id: first("profile_id"),
                instance_id: first("instance_id"),
                locale_name: first("locale_name"),
                ui_language: first("ui_language"),
                region: first("region"),
                tz_windows: first("tz_windows"),
                tz_iana: first("tz_iana"),
                inherit_children: get_flag("inherit_children"),
                audit: get_flag("audit"),
                dns_mode: get_flag("dns_mode"),
                dns_servers: list("dns_server"),
                registry_paths: list("registry_path"),
                webrtc: first("webrtc"),
            },
            "BIND_PID" => IpcMessage::BindPid {
                pid: get_u32("pid")?,
                profile_id: first("profile_id"),
                parent_pid: first("parent_pid").parse().unwrap_or(0),
            },
            other => IpcMessage::Other {
                name: other.to_string(),
                fields: map,
            },
        })
    }
}

/// Convert a domain Profile into a PROFILE message.
pub fn profile_to_message(profile: &EnvironmentProfile, instance_id: &str) -> IpcMessage {
    profile_to_message_with_flags(profile, instance_id, true, false)
}

/// PROFILE message with explicit inherit/audit flags from the Run request.
pub fn profile_to_message_with_flags(
    profile: &EnvironmentProfile,
    instance_id: &str,
    inherit_children: bool,
    audit: bool,
) -> IpcMessage {
    IpcMessage::Profile {
        profile_id: profile.id.to_string(),
        instance_id: instance_id.to_string(),
        locale_name: profile.locale.locale_name.clone(),
        ui_language: profile.locale.ui_language.clone(),
        region: profile.locale.region.clone(),
        tz_windows: profile.timezone.windows_id.clone(),
        tz_iana: profile.timezone.iana_id.clone(),
        inherit_children,
        audit,
        dns_mode: matches!(profile.dns.mode, DnsMode::VirtualView),
        dns_servers: profile
            .dns
            .servers
            .iter()
            .map(|s| s.to_string())
            .collect(),
        registry_paths: profile.registry.whitelist_paths.clone(),
        webrtc: profile.browser.webrtc.as_str().to_string(),
    }
}

/// Convert a PROFILE message back into a domain Profile.
pub fn message_to_profile(msg: &IpcMessage) -> Result<EnvironmentProfile, IpcError> {
    let IpcMessage::Profile {
        locale_name,
        ui_language,
        region,
        tz_windows,
        tz_iana,
        dns_mode,
        dns_servers,
        registry_paths,
        webrtc,
        ..
    } = msg
    else {
        return Err(IpcError::Protocol("not a PROFILE message".into()));
    };
    if locale_name.is_empty() || ui_language.is_empty() || region.is_empty() || tz_windows.is_empty()
    {
        return Err(IpcError::Protocol(
            "PROFILE missing required fields (locale_name/ui_language/region/tz_windows)".into(),
        ));
    }
    let mut servers = Vec::new();
    for s in dns_servers {
        if let Ok(ip) = s.parse() {
            servers.push(ip);
        }
    }
    let webrtc_policy = if webrtc.is_empty() {
        // Older peers omit the field: Host (no browser alteration).
        envbox_core::WebRtcPolicy::Host
    } else {
        envbox_core::WebRtcPolicy::parse(webrtc).ok_or_else(|| {
            IpcError::Protocol(format!("PROFILE invalid webrtc={webrtc:?}"))
        })?
    };
    Ok(EnvironmentProfile {
        id: Uuid::nil(),
        name: "ipc".into(),
        locale: LocaleProfile {
            locale_name: locale_name.clone(),
            ui_language: ui_language.clone(),
            region: region.clone(),
        },
        timezone: TimezoneProfile {
            windows_id: tz_windows.clone(),
            iana_id: tz_iana.clone(),
        },
        dns: envbox_core::DnsProfile {
            mode: if *dns_mode {
                DnsMode::VirtualView
            } else {
                DnsMode::Host
            },
            servers,
        },
        environment: HashMap::new(),
        registry: RegistryProfile {
            whitelist_paths: registry_paths.clone(),
        },
        browser: envbox_core::BrowserPrivacyProfile {
            webrtc: webrtc_policy,
        },
    })
}

/// Quote a value if it may contain spaces / quotes / be empty.
fn quote_value(v: &str) -> String {
    let needs = v.is_empty()
        || v.contains(' ')
        || v.contains('\t')
        || v.contains('"')
        || v.contains('\\')
        || v.contains('\n')
        || v.contains('\r');
    if !needs {
        return v.to_string();
    }
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Tokenize `MSG key=value key="quoted value"`.
fn tokenize_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut esc = false;
    for c in line.chars() {
        if esc {
            match c {
                '\\' => cur.push('\\'),
                '"' => cur.push('"'),
                'n' => cur.push('\n'),
                'r' => cur.push('\r'),
                't' => cur.push('\t'),
                other => cur.push(other),
            }
            esc = false;
            continue;
        }
        match c {
            '\\' if in_quotes => esc = true,
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Host-side Session Registry used by the IPC / Broker server.
///
/// Maps PID → Profile and tracks process membership for one or more
/// Environment Sessions (V0.3 ticket 43).
#[derive(Default)]
pub struct SessionTable {
    /// profile_id → PROFILE message
    profiles: HashMap<String, IpcMessage>,
    /// pid → profile_id
    bindings: HashMap<u32, String>,
    /// pid → parent pid (session membership / Process Tracker)
    parents: HashMap<u32, u32>,
    /// live pids per profile_id
    live: HashMap<String, HashSet<u32>>,
    instance_id: String,
    /// lifecycle notices (best-effort log for broker observability)
    pub events: Vec<IpcMessage>,
}

impl SessionTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_instance_id(&mut self, id: &str) {
        self.instance_id = id.to_string();
    }

    pub fn register_profile(&mut self, profile: &EnvironmentProfile) {
        let msg = profile_to_message(profile, &self.instance_id);
        self.profiles.insert(profile.id.to_string(), msg);
    }

    pub fn register_profile_flags(
        &mut self,
        profile: &EnvironmentProfile,
        inherit_children: bool,
        audit: bool,
    ) {
        let msg = profile_to_message_with_flags(
            profile,
            &self.instance_id,
            inherit_children,
            audit,
        );
        self.profiles.insert(profile.id.to_string(), msg);
    }

    pub fn bind_pid(&mut self, pid: u32, profile_id: &str) {
        self.bindings.insert(pid, profile_id.to_string());
        self.live
            .entry(profile_id.to_string())
            .or_default()
            .insert(pid);
    }

    pub fn register_profile_message(&mut self, msg: IpcMessage) {
        if let IpcMessage::RegisterProfile { profile_id, .. } = &msg {
            let profile_id = profile_id.clone();
            let as_profile = match msg {
                IpcMessage::RegisterProfile {
                    profile_id,
                    instance_id,
                    locale_name,
                    ui_language,
                    region,
                    tz_windows,
                    tz_iana,
                    inherit_children,
                    audit,
                    dns_mode,
                    dns_servers,
                    registry_paths,
                    webrtc,
                } => IpcMessage::Profile {
                    profile_id,
                    instance_id,
                    locale_name,
                    ui_language,
                    region,
                    tz_windows,
                    tz_iana,
                    inherit_children,
                    audit,
                    dns_mode,
                    dns_servers,
                    registry_paths,
                    webrtc,
                },
                other => other,
            };
            self.profiles.insert(profile_id, as_profile);
        }
    }

    /// Live process set for a profile (Process Tracker).
    pub fn live_pids(&self, profile_id: &str) -> Vec<u32> {
        self.live
            .get(profile_id)
            .map(|s| {
                let mut v: Vec<u32> = s.iter().copied().collect();
                v.sort_unstable();
                v
            })
            .unwrap_or_default()
    }

    pub fn profile_of(&self, pid: u32) -> Option<&str> {
        self.bindings.get(&pid).map(String::as_str)
    }

    pub fn handle(&mut self, msg: &IpcMessage) -> Option<IpcMessage> {
        match msg {
            IpcMessage::Hello { .. } => {
                self.events.push(msg.clone());
                None
            }
            IpcMessage::GetProfile { pid, profile_id } => {
                let key = if profile_id.is_empty() {
                    self.bindings.get(pid).cloned().unwrap_or_default()
                } else {
                    profile_id.clone()
                };
                // Also accept parent binding: child inherits parent's profile.
                let key = if key.is_empty() {
                    self.parents
                        .get(pid)
                        .and_then(|pp| self.bindings.get(pp).cloned())
                        .unwrap_or_default()
                } else {
                    key
                };
                if !key.is_empty() {
                    self.bindings.insert(*pid, key.clone());
                    self.live.entry(key.clone()).or_default().insert(*pid);
                }
                self.profiles.get(&key).cloned().or_else(|| {
                    // Empty PROFILE fails closed in Runtime (required fields).
                    Some(IpcMessage::Profile {
                        profile_id: key,
                        instance_id: self.instance_id.clone(),
                        locale_name: String::new(),
                        ui_language: String::new(),
                        region: String::new(),
                        tz_windows: String::new(),
                        tz_iana: String::new(),
                        inherit_children: true,
                        audit: false,
                        dns_mode: false,
                        dns_servers: vec![],
                        registry_paths: vec![],
                        webrtc: "host".into(),
                    })
                })
            }
            IpcMessage::RegisterProfile { .. } => {
                self.register_profile_message(msg.clone());
                None
            }
            IpcMessage::BindPid {
                pid,
                profile_id,
                parent_pid,
            } => {
                if *parent_pid != 0 {
                    self.parents.insert(*pid, *parent_pid);
                }
                if !profile_id.is_empty() {
                    self.bind_pid(*pid, profile_id);
                } else if *parent_pid != 0 {
                    if let Some(p) = self.bindings.get(parent_pid).cloned() {
                        self.bind_pid(*pid, &p);
                    }
                }
                None
            }
            IpcMessage::ProcessCreated {
                pid,
                child_pid,
                image: _,
            } => {
                self.parents.insert(*child_pid, *pid);
                if let Some(p) = self.bindings.get(pid).cloned() {
                    self.bind_pid(*child_pid, &p);
                }
                self.events.push(msg.clone());
                None
            }
            IpcMessage::ProcessExited { pid, .. } => {
                if let Some(key) = self.bindings.get(pid).cloned() {
                    if let Some(set) = self.live.get_mut(&key) {
                        set.remove(pid);
                    }
                }
                self.bindings.remove(pid);
                self.parents.remove(pid);
                self.events.push(msg.clone());
                None
            }
            IpcMessage::RuntimeReady { .. } | IpcMessage::HookError { .. } => {
                self.events.push(msg.clone());
                None
            }
            _ => None,
        }
    }
}

/// In-process fake broker for tests (no OS pipe).
pub struct FakeBroker {
    pub table: SessionTable,
    pub seen: Vec<IpcMessage>,
}

impl FakeBroker {
    pub fn new() -> Self {
        Self {
            table: SessionTable::new(),
            seen: Vec::new(),
        }
    }

    pub fn send(&mut self, msg: IpcMessage) -> Option<IpcMessage> {
        self.seen.push(msg.clone());
        self.table.handle(&msg)
    }
}

impl Default for FakeBroker {
    fn default() -> Self {
        Self::new()
    }
}

/// Round-trip helper used by unit tests.
pub fn round_trip(msg: &IpcMessage) -> IpcMessage {
    IpcMessage::decode_line(&msg.encode_line()).expect("protocol round-trip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trip() {
        let msg = IpcMessage::Hello {
            pid: 42,
            instance_id: "abc-def".into(),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn get_profile_round_trip() {
        let msg = IpcMessage::GetProfile {
            pid: 7,
            profile_id: "p1".into(),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn all_public_message_names_encode() {
        let msgs = [
            IpcMessage::Hello {
                pid: 1,
                instance_id: "x".into(),
            },
            IpcMessage::GetProfile {
                pid: 1,
                profile_id: "x".into(),
            },
            IpcMessage::Profile {
                profile_id: "x".into(),
                instance_id: "i".into(),
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
                tz_windows: "Pacific Standard Time".into(),
                tz_iana: "America/Los_Angeles".into(),
                inherit_children: true,
                audit: false,
                dns_mode: true,
                dns_servers: vec!["1.1.1.1".into()],
                registry_paths: vec!["HKCU\\Software\\EnvBox".into()],
                webrtc: "proxy_only".into(),
            },
            IpcMessage::RuntimeReady { pid: 1 },
            IpcMessage::HookError {
                pid: 1,
                api: "GetTimeZoneInformation".into(),
                code: 5,
                detail: "access denied".into(),
            },
            IpcMessage::ProcessCreated {
                pid: 1,
                child_pid: 2,
                image: r"C:\a b\app.exe".into(),
            },
            IpcMessage::ProcessExited {
                pid: 2,
                exit_code: 0,
            },
        ];
        for m in msgs {
            assert_eq!(round_trip(&m), m, "round-trip failed for {}", m.name());
        }
    }

    #[test]
    fn quoted_values_with_spaces_and_escapes() {
        let msg = IpcMessage::HookError {
            pid: 1,
            api: "GetTimeZoneInformation".into(),
            code: 5,
            detail: "a b=c \"x\"".into(),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn profile_message_round_trip() {
        let profile = EnvironmentProfile {
            id: Uuid::nil(),
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
            dns: envbox_core::DnsProfile {
                mode: DnsMode::VirtualView,
                servers: vec!["1.1.1.1".parse().unwrap()],
            },
            environment: HashMap::new(),
            registry: RegistryProfile {
                whitelist_paths: vec!["HKCU\\Software\\EnvBox".into()],
            },
            browser: Default::default(),
        };
        let msg = profile_to_message(&profile, "inst-1");
        let back = round_trip(&msg);
        let decoded = message_to_profile(&back).unwrap();
        assert_eq!(decoded.locale, profile.locale);
        assert_eq!(decoded.timezone, profile.timezone);
        assert_eq!(decoded.dns.servers, profile.dns.servers);
        assert_eq!(
            decoded.registry.whitelist_paths,
            profile.registry.whitelist_paths
        );
        assert_eq!(decoded.browser.webrtc, profile.browser.webrtc);
    }

    #[test]
    fn profile_webrtc_round_trips_and_invalid_rejected() {
        let mut profile = EnvironmentProfile {
            id: Uuid::nil(),
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
            dns: envbox_core::DnsProfile {
                mode: DnsMode::Host,
                servers: vec![],
            },
            environment: HashMap::new(),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        };
        profile.browser.webrtc = envbox_core::WebRtcPolicy::Strict;
        let msg = profile_to_message(&profile, "inst-1");
        let decoded = message_to_profile(&msg).unwrap();
        assert_eq!(decoded.browser.webrtc, envbox_core::WebRtcPolicy::Strict);

        // Empty webrtc (older peer) → Host.
        if let IpcMessage::Profile { webrtc, .. } = &msg {
            let mut m = msg.clone();
            if let IpcMessage::Profile { webrtc: w, .. } = &mut m {
                *w = String::new();
            }
            let _ = webrtc;
            let decoded = message_to_profile(&m).unwrap();
            assert_eq!(decoded.browser.webrtc, envbox_core::WebRtcPolicy::Host);

            // Garbage webrtc → protocol error (never silent Host).
            if let IpcMessage::Profile { webrtc: w, .. } = &mut m {
                *w = "nope".into();
            }
            assert!(message_to_profile(&m).is_err());
        }
    }

    #[test]
    fn fake_broker_serves_profile() {
        let profile = EnvironmentProfile {
            id: Uuid::nil(),
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
            dns: envbox_core::DnsProfile {
                mode: DnsMode::Host,
                servers: vec![],
            },
            environment: HashMap::new(),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        };
        let mut broker = FakeBroker::new();
        broker.table.set_instance_id("inst");
        // Use a fixed id so bind_pid can resolve it.
        let mut p = profile;
        p.id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        broker.table.register_profile(&p);
        broker.table.bind_pid(10, &p.id.to_string());
        let reply = broker
            .send(IpcMessage::GetProfile {
                pid: 10,
                profile_id: String::new(),
            })
            .expect("profile reply");
        assert_eq!(reply.name(), "PROFILE");
    }

    #[test]
    fn profile_requires_core_fields() {
        let msg = IpcMessage::Profile {
            profile_id: "x".into(),
            instance_id: "i".into(),
            locale_name: String::new(),
            ui_language: "en-US".into(),
            region: "US".into(),
            tz_windows: "PST".into(),
            tz_iana: "UTC".into(),
            inherit_children: true,
            audit: false,
            dns_mode: false,
            dns_servers: vec![],
            registry_paths: vec![],
            webrtc: "host".into(),
        };
        assert!(message_to_profile(&msg).is_err());
    }

    #[test]
    fn register_and_bind_round_trip() {
        let reg = IpcMessage::RegisterProfile {
            profile_id: "p1".into(),
            instance_id: "i".into(),
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
            tz_windows: "Pacific Standard Time".into(),
            tz_iana: "America/Los_Angeles".into(),
            inherit_children: true,
            audit: false,
            dns_mode: false,
            dns_servers: vec![],
            registry_paths: vec![],
            webrtc: "strict".into(),
        };
        assert_eq!(round_trip(&reg), reg);
        let bind = IpcMessage::BindPid {
            pid: 10,
            profile_id: "p1".into(),
            parent_pid: 1,
        };
        assert_eq!(round_trip(&bind), bind);
    }

    #[test]
    fn session_registry_tracks_children_and_exit() {
        let mut t = SessionTable::new();
        t.handle(&IpcMessage::RegisterProfile {
            profile_id: "p1".into(),
            instance_id: "i".into(),
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
            tz_windows: "PST".into(),
            tz_iana: "UTC".into(),
            inherit_children: true,
            audit: false,
            dns_mode: false,
            dns_servers: vec![],
            registry_paths: vec![],
            webrtc: "host".into(),
        });
        t.handle(&IpcMessage::BindPid {
            pid: 10,
            profile_id: "p1".into(),
            parent_pid: 0,
        });
        t.handle(&IpcMessage::ProcessCreated {
            pid: 10,
            child_pid: 11,
            image: "child.exe".into(),
        });
        assert_eq!(t.profile_of(11), Some("p1"));
        assert_eq!(t.live_pids("p1"), vec![10, 11]);
        t.handle(&IpcMessage::ProcessExited {
            pid: 11,
            exit_code: 0,
        });
        assert_eq!(t.live_pids("p1"), vec![10]);
    }
}
