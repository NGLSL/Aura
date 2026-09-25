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
//! Win32 keeps ENVBOX_* + profiles.toml as fallback; IPC is preferred for
//! packaged roots that have no Environment Block.

use envbox_core::{DnsMode, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile};
use std::collections::HashMap;
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
            } => {
                let mut s = format!(
                    "PROFILE profile_id={} instance_id={} locale_name={} ui_language={} region={} tz_windows={} tz_iana={} inherit_children={} audit={} dns_mode={}",
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
            other => IpcMessage::Other {
                name: other.to_string(),
                fields: map,
            },
        })
    }
}

/// Convert a domain Profile into a PROFILE message.
pub fn profile_to_message(profile: &EnvironmentProfile, instance_id: &str) -> IpcMessage {
    IpcMessage::Profile {
        profile_id: profile.id.to_string(),
        instance_id: instance_id.to_string(),
        locale_name: profile.locale.locale_name.clone(),
        ui_language: profile.locale.ui_language.clone(),
        region: profile.locale.region.clone(),
        tz_windows: profile.timezone.windows_id.clone(),
        tz_iana: profile.timezone.iana_id.clone(),
        inherit_children: true,
        audit: false,
        dns_mode: matches!(profile.dns.mode, DnsMode::VirtualView),
        dns_servers: profile
            .dns
            .servers
            .iter()
            .map(|s| s.to_string())
            .collect(),
        registry_paths: profile.registry.whitelist_paths.clone(),
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

/// Host-side session table used by the IPC server.
#[derive(Default)]
pub struct SessionTable {
    /// profile_id → PROFILE message
    profiles: HashMap<String, IpcMessage>,
    /// pid → profile_id
    bindings: HashMap<u32, String>,
    instance_id: String,
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

    pub fn bind_pid(&mut self, pid: u32, profile_id: &str) {
        self.bindings.insert(pid, profile_id.to_string());
    }

    pub fn handle(&mut self, msg: &IpcMessage) -> Option<IpcMessage> {
        match msg {
            IpcMessage::Hello { .. } => None,
            IpcMessage::GetProfile { pid, profile_id } => {
                let key = if profile_id.is_empty() {
                    self.bindings.get(pid).cloned().unwrap_or_default()
                } else {
                    profile_id.clone()
                };
                self.profiles.get(&key).cloned().or_else(|| {
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
                    })
                })
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
        };
        assert!(message_to_profile(&msg).is_err());
    }
}
