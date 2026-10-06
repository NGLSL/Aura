//! Runtime ↔ Host IPC protocol (packaged-v1 ticket 40).
//!
//! Wire format matches `runtime/src/ipc_bootstrap.h`:
//! one line per message, UTF-8, `MSG_NAME key=value ...`, values are bare
//! tokens or double-quoted strings (`\\` `\"` `\n` `\r` `\t` escapes).
//! Lists use repeated keys (`dns_server=`, `registry_path=`, `environment=`).
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
pub const IPC_MAX_LINE_BYTES: usize = 8192;
pub const IPC_IDENTITY_MAX_LINE_BYTES: usize = 32768;
pub const RUNTIME_IDENTITY_PROTOCOL: u32 = 1;

/// Facts observed after the Runtime's hook transaction committed. Counts
/// describe attached APIs, never complete Windows/process-tree coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub pid: u32,
    pub creation_time: u64,
    pub protocol: u32,
    pub runtime_version: String,
    pub module_path: String,
    pub actual_profile: String,
    pub config_complete: bool,
    pub hooks: Vec<(String, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedRuntimeIdentity {
    pub identity: RuntimeIdentity,
    pub module_sha256: String,
    pub config_sha256: String,
}
pub const RUNTIME_ENVIRONMENT_MAX: usize = 32;
pub const RUNTIME_ENVIRONMENT_ENTRY_MAX_BYTES: usize = 512;

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
    RegisterChild {
        pid: u32,
        creation_time: u64,
        instance_id: String,
        profile_id: String,
        child_pid: u32,
        child_creation_time: u64,
    },
    ChildBound {
        pid: u32,
        creation_time: u64,
    },
    RuntimeIdentity(RuntimeIdentity),
    RuntimeReconnect(RuntimeIdentity),
    RuntimeIdentityConfirm(RuntimeIdentity),
    RuntimeIdentityConfirmed {
        pid: u32,
        creation_time: u64,
    },
    RuntimeReconnected {
        pid: u32,
        creation_time: u64,
    },
    RuntimeReconnectChallenge {
        pid: u32,
        creation_time: u64,
        nonce: String,
    },
    RuntimeReconnectProof {
        pid: u32,
        creation_time: u64,
        nonce: String,
    },
    StartupGateReady {
        pid: u32,
        creation_time: u64,
    },
    StartupRelease {
        pid: u32,
        creation_time: u64,
    },
    StartupGateReleased {
        pid: u32,
        creation_time: u64,
    },
    StartupGateConfirmed {
        pid: u32,
        creation_time: u64,
    },
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
        dns_config: Option<envbox_core::DnsProfile>,
        registry_paths: Vec<String>,
        /// Profile environment overrides encoded as ordered key/value pairs.
        environment: Vec<(String, String)>,
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
        dns_config: Option<envbox_core::DnsProfile>,
        registry_paths: Vec<String>,
        environment: Vec<(String, String)>,
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
            IpcMessage::RegisterChild { .. } => "REGISTER_CHILD",
            IpcMessage::ChildBound { .. } => "CHILD_BOUND",
            IpcMessage::RuntimeIdentity(_) => "RUNTIME_IDENTITY",
            IpcMessage::RuntimeReconnect(_) => "RUNTIME_RECONNECT",
            IpcMessage::RuntimeIdentityConfirm(_) => "RUNTIME_IDENTITY_CONFIRM",
            IpcMessage::RuntimeIdentityConfirmed { .. } => "RUNTIME_IDENTITY_CONFIRMED",
            IpcMessage::RuntimeReconnected { .. } => "RUNTIME_RECONNECTED",
            IpcMessage::RuntimeReconnectChallenge { .. } => "RUNTIME_RECONNECT_CHALLENGE",
            IpcMessage::RuntimeReconnectProof { .. } => "RUNTIME_RECONNECT_PROOF",
            IpcMessage::StartupGateReady { .. } => "STARTUP_GATE_READY",
            IpcMessage::StartupRelease { .. } => "STARTUP_RELEASE",
            IpcMessage::StartupGateReleased { .. } => "STARTUP_GATE_RELEASED",
            IpcMessage::StartupGateConfirmed { .. } => "STARTUP_GATE_CONFIRMED",
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
            IpcMessage::RuntimeReconnectChallenge { pid, creation_time, nonce }
            | IpcMessage::RuntimeReconnectProof { pid, creation_time, nonce } => format!("{} pid={pid} creation_time={creation_time} nonce={}", self.name(), quote_value(nonce)),
            IpcMessage::RegisterChild { pid, creation_time, instance_id, profile_id, child_pid, child_creation_time } => format!("REGISTER_CHILD pid={pid} creation_time={creation_time} instance_id={} profile_id={} child_pid={child_pid} child_creation_time={child_creation_time}", quote_value(instance_id), quote_value(profile_id)),
            IpcMessage::ChildBound { pid, creation_time } => format!("CHILD_BOUND pid={pid} creation_time={creation_time}"),
            IpcMessage::StartupGateReady { pid, creation_time }
            | IpcMessage::StartupRelease { pid, creation_time }
            | IpcMessage::StartupGateReleased { pid, creation_time }
            | IpcMessage::StartupGateConfirmed { pid, creation_time }
            | IpcMessage::RuntimeReconnected { pid, creation_time } => {
                format!("{} pid={pid} creation_time={creation_time}", self.name())
            }
            IpcMessage::RuntimeIdentityConfirmed { pid, creation_time } => format!("{} pid={pid} creation_time={creation_time}",self.name()),
            IpcMessage::RuntimeIdentity(id) | IpcMessage::RuntimeReconnect(id) | IpcMessage::RuntimeIdentityConfirm(id) => {
                let mut line = format!("{} pid={} creation_time={} protocol={} version={} module_path={} actual_profile={} config_complete={}", self.name(), id.pid, id.creation_time, id.protocol, quote_value(&id.runtime_version), quote_value(&id.module_path), quote_value(&id.actual_profile), u8::from(id.config_complete));
                for (group, count) in &id.hooks {
                    line.push_str(&format!(
                        " hook={}",
                        quote_value(&format!("{group}:{count}"))
                    ));
                }
                line
            }
            IpcMessage::Hello { pid, instance_id } => {
                format!("HELLO pid={pid} instance_id={}", quote_value(instance_id))
            }
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
                dns_config,
                registry_paths,
                environment,
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
                append_dns_fields(&mut s, dns_config.as_ref());
                for p in registry_paths {
                    s.push_str(&format!(" registry_path={}", quote_value(p)));
                }
                for (key, value) in environment {
                    s.push_str(&format!(
                        " environment={}",
                        quote_value(&format!("{key}={value}"))
                    ));
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
                let mut s = format!("HOOK_ERROR pid={pid} api={} code={code}", quote_value(api));
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
                dns_config,
                registry_paths,
                environment,
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
                append_dns_fields(&mut s, dns_config.as_ref());
                for p in registry_paths {
                    s.push_str(&format!(" registry_path={}", quote_value(p)));
                }
                for (key, value) in environment {
                    s.push_str(&format!(
                        " environment={}",
                        quote_value(&format!("{key}={value}"))
                    ));
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
        let tokens = tokenize_line(line)?;
        if tokens.is_empty() {
            return Err(IpcError::Protocol("empty message".into()));
        }
        let name = tokens[0].clone();
        let maximum = if name == "RUNTIME_IDENTITY"
            || name == "RUNTIME_RECONNECT"
            || name == "RUNTIME_IDENTITY_CONFIRM"
        {
            IPC_IDENTITY_MAX_LINE_BYTES
        } else {
            IPC_MAX_LINE_BYTES
        };
        if line.len() >= maximum {
            return Err(IpcError::Protocol("IPC message exceeds wire limit".into()));
        }
        let mut map: Vec<(String, String)> = Vec::new();
        let mut lists: HashMap<String, Vec<String>> = HashMap::new();
        for tok in &tokens[1..] {
            if let Some((k, v)) = tok.split_once('=') {
                if k.is_empty()
                    || (!matches!(k, "dns_server" | "registry_path" | "environment" | "hook")
                        && lists.contains_key(k))
                {
                    return Err(IpcError::Protocol(format!("empty or duplicated field {k}")));
                }
                lists.entry(k.to_string()).or_default().push(v.to_string());
                map.push((k.to_string(), v.to_string()));
            } else {
                return Err(IpcError::Protocol("field missing '='".into()));
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
            "REGISTER_CHILD" => IpcMessage::RegisterChild {
                pid: get_u32("pid")?,
                creation_time: first("creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid creation_time".into()))?,
                instance_id: first("instance_id"),
                profile_id: first("profile_id"),
                child_pid: get_u32("child_pid")?,
                child_creation_time: first("child_creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid child_creation_time".into()))?,
            },
            "CHILD_BOUND" => IpcMessage::ChildBound {
                pid: get_u32("pid")?,
                creation_time: first("creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid creation_time".into()))?,
            },
            "STARTUP_GATE_READY"
            | "STARTUP_RELEASE"
            | "STARTUP_GATE_RELEASED"
            | "STARTUP_GATE_CONFIRMED" => {
                let pid = get_u32("pid")?;
                let creation_time = first("creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid gate creation_time".into()))?;
                match name.as_str() {
                    "STARTUP_GATE_READY" => IpcMessage::StartupGateReady { pid, creation_time },
                    "STARTUP_RELEASE" => IpcMessage::StartupRelease { pid, creation_time },
                    "STARTUP_GATE_RELEASED" => {
                        IpcMessage::StartupGateReleased { pid, creation_time }
                    }
                    _ => IpcMessage::StartupGateConfirmed { pid, creation_time },
                }
            }
            "RUNTIME_RECONNECT_CHALLENGE" | "RUNTIME_RECONNECT_PROOF" => {
                let pid = get_u32("pid")?;
                let creation_time = first("creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid creation_time".into()))?;
                let nonce = first("nonce");
                if name == "RUNTIME_RECONNECT_CHALLENGE" {
                    IpcMessage::RuntimeReconnectChallenge {
                        pid,
                        creation_time,
                        nonce,
                    }
                } else {
                    IpcMessage::RuntimeReconnectProof {
                        pid,
                        creation_time,
                        nonce,
                    }
                }
            }
            "RUNTIME_RECONNECTED" => IpcMessage::RuntimeReconnected {
                pid: get_u32("pid")?,
                creation_time: first("creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid creation_time".into()))?,
            },
            "RUNTIME_IDENTITY_CONFIRMED" => IpcMessage::RuntimeIdentityConfirmed {
                pid: get_u32("pid")?,
                creation_time: first("creation_time")
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid creation_time".into()))?,
            },
            "RUNTIME_IDENTITY" | "RUNTIME_RECONNECT" | "RUNTIME_IDENTITY_CONFIRM" => {
                let id = RuntimeIdentity {
                    pid: get_u32("pid")?,
                    creation_time: first("creation_time")
                        .parse()
                        .map_err(|_| IpcError::Protocol("invalid creation_time".into()))?,
                    protocol: get_u32("protocol")?,
                    runtime_version: first("version"),
                    module_path: first("module_path"),
                    actual_profile: first("actual_profile"),
                    config_complete: get_flag("config_complete"),
                    hooks: list("hook")
                        .into_iter()
                        .map(|entry| {
                            let (key, count) = entry
                                .split_once(':')
                                .ok_or_else(|| IpcError::Protocol("invalid hook count".into()))?;
                            Ok((
                                key.to_string(),
                                count
                                    .parse()
                                    .map_err(|_| IpcError::Protocol("invalid hook count".into()))?,
                            ))
                        })
                        .collect::<Result<Vec<_>, IpcError>>()?,
                };
                if name == "RUNTIME_IDENTITY_CONFIRM" {
                    IpcMessage::RuntimeIdentityConfirm(id)
                } else if name == "RUNTIME_RECONNECT" {
                    IpcMessage::RuntimeReconnect(id)
                } else {
                    IpcMessage::RuntimeIdentity(id)
                }
            }
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
                dns_config: decode_dns_fields(&map)?,
                registry_paths: list("registry_path"),
                environment: list("environment")
                    .into_iter()
                    .filter_map(|entry| {
                        entry
                            .split_once('=')
                            .map(|(key, value)| (key.to_string(), value.to_string()))
                    })
                    .collect(),
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
                dns_config: decode_dns_fields(&map)?,
                registry_paths: list("registry_path"),
                environment: list("environment")
                    .into_iter()
                    .filter_map(|entry| {
                        entry
                            .split_once('=')
                            .map(|(key, value)| (key.to_string(), value.to_string()))
                    })
                    .collect(),
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
    let mut environment: Vec<(String, String)> = profile
        .environment
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    environment.sort_by(|a, b| a.0.cmp(&b.0));
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
        dns_servers: Vec::new(),
        dns_config: Some(envbox_core::DnsProfile::typed(
            profile.dns.mode.clone(),
            profile.dns.strict,
            profile.dns.effective_upstreams(),
        )),
        registry_paths: profile.registry.whitelist_paths.clone(),
        environment,
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
        dns_config,
        registry_paths,
        environment,
        webrtc,
        ..
    } = msg
    else {
        return Err(IpcError::Protocol("not a PROFILE message".into()));
    };
    if locale_name.is_empty()
        || ui_language.is_empty()
        || region.is_empty()
        || tz_windows.is_empty()
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
        envbox_core::WebRtcPolicy::parse(webrtc)
            .ok_or_else(|| IpcError::Protocol(format!("PROFILE invalid webrtc={webrtc:?}")))?
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
        dns: dns_config.clone().unwrap_or_else(|| {
            envbox_core::DnsProfile::from_servers(
                if *dns_mode {
                    DnsMode::VirtualView
                } else {
                    DnsMode::Host
                },
                servers,
            )
        }),
        environment: environment.iter().cloned().collect(),
        registry: RegistryProfile {
            whitelist_paths: registry_paths.clone(),
        },
        browser: envbox_core::BrowserPrivacyProfile {
            webrtc: webrtc_policy,
        },
    })
}

/// Quote a value if it may contain spaces / quotes / be empty.
fn append_dns_fields(line: &mut String, config: Option<&envbox_core::DnsProfile>) {
    if let Some(config) = config {
        match config.flat_fields() {
            Ok(fields) => {
                for (key, value) in fields {
                    line.push_str(&format!(" {key}={}", quote_value(&value)));
                }
            }
            Err(_) => line.push_str(" dns_config_version=invalid"),
        }
    }
}

fn decode_dns_fields(
    fields: &[(String, String)],
) -> Result<Option<envbox_core::DnsProfile>, IpcError> {
    use envbox_core::{DnsProfile, DnsUpstream};
    let value = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let required =
        |key: &str| value(key).ok_or_else(|| IpcError::Protocol(format!("missing {key}")));
    let typed = fields
        .iter()
        .any(|(key, _)| key == "dns_strict" || key.starts_with("dns_upstream"));
    let Some(version) = value("dns_config_version") else {
        if typed {
            return Err(IpcError::Protocol(
                "typed DNS fields missing version".into(),
            ));
        }
        return Ok(None);
    };
    if version != "1"
        || fields
            .iter()
            .any(|(key, _)| key == "dns_server" || key == "dns_servers")
    {
        return Err(IpcError::Protocol(
            "unsupported or mixed DNS configuration".into(),
        ));
    }
    let mode = match required("dns_mode")? {
        "0" => DnsMode::Host,
        "1" => DnsMode::VirtualView,
        _ => return Err(IpcError::Protocol("invalid DNS mode".into())),
    };
    let strict = match required("dns_strict")? {
        "0" => false,
        "1" => true,
        _ => return Err(IpcError::Protocol("invalid DNS strict flag".into())),
    };
    let count: usize = required("dns_upstream_count")?
        .parse()
        .map_err(|_| IpcError::Protocol("invalid upstream count".into()))?;
    if count > 8 {
        return Err(IpcError::Protocol("too many DNS upstreams".into()));
    }
    let mut upstreams = Vec::new();
    for i in 0..count {
        let field = |name: &str| required(&format!("dns_upstream_{i}_{name}"));
        let endpoint = || -> Result<_, IpcError> {
            Ok((
                field("address")?
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid DNS literal address".into()))?,
                field("port")?
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid DNS port".into()))?,
            ))
        };
        upstreams.push(match field("type")? {
            "udp" => {
                let (address, port) = endpoint()?;
                DnsUpstream::Udp { address, port }
            }
            "tcp" => {
                let (address, port) = endpoint()?;
                DnsUpstream::Tcp { address, port }
            }
            "dot" => {
                let (address, port) = endpoint()?;
                DnsUpstream::Dot {
                    address,
                    port,
                    server_name: field("server_name")?.into(),
                }
            }
            "doh" => {
                let count: usize = field("bootstrap_count")?
                    .parse()
                    .map_err(|_| IpcError::Protocol("invalid bootstrap count".into()))?;
                if count > 8 {
                    return Err(IpcError::Protocol("too many bootstrap IPs".into()));
                }
                let mut bootstrap_ips = Vec::new();
                for b in 0..count {
                    bootstrap_ips.push(
                        field(&format!("bootstrap_{b}"))?
                            .parse()
                            .map_err(|_| IpcError::Protocol("invalid bootstrap IP".into()))?,
                    );
                }
                DnsUpstream::Doh {
                    url: field("url")?.into(),
                    bootstrap_ips,
                    tls_revocation: match field("tls_revocation")? {
                        "0" => envbox_core::DnsTlsRevocation::Standard,
                        "1" => envbox_core::DnsTlsRevocation::StrictOffline,
                        _ => {
                            return Err(IpcError::Protocol(
                                "invalid DoH TLS revocation policy".into(),
                            ))
                        }
                    },
                }
            }
            _ => return Err(IpcError::Protocol("unknown DNS upstream protocol".into())),
        });
    }
    let config = DnsProfile::typed(mode, strict, upstreams);
    config
        .validate()
        .map_err(|e| IpcError::Protocol(e.to_string()))?;
    let canonical = config
        .flat_fields()
        .map_err(|e| IpcError::Protocol(e.to_string()))?;
    if fields
        .iter()
        .filter(|(key, _)| key.starts_with("dns_") && key != "dns_mode")
        .any(|entry| !canonical.contains(entry))
    {
        return Err(IpcError::Protocol(
            "unexpected or noncanonical typed DNS field".into(),
        ));
    }
    Ok(Some(config))
}

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
fn tokenize_line(line: &str) -> Result<Vec<String>, IpcError> {
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
    if in_quotes || esc {
        return Err(IpcError::Protocol("unterminated quoted field".into()));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
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
    generations: HashMap<u32, u64>,
    expected_runtimes: HashMap<u32, (std::path::PathBuf, String)>,
    runtime_bundles: HashMap<u32, HashMap<String, (std::path::PathBuf, String)>>,
    identities: HashMap<u32, ObservedRuntimeIdentity>,
    // Only root observations survive exit, bounded by the session's roots.
    exited_root_identities: HashMap<(u32, u64), ObservedRuntimeIdentity>,
    reconnect_challenges: HashMap<u32, String>,
    reconfirmed: HashMap<u32, u64>,
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
        let msg =
            profile_to_message_with_flags(profile, &self.instance_id, inherit_children, audit);
        self.profiles.insert(profile.id.to_string(), msg);
    }

    pub fn bind_pid(&mut self, pid: u32, profile_id: &str) {
        // Immutable binding: a repeated LoadLibrary or management retry cannot
        // turn an existing process into another Profile/Instance.
        if self
            .bindings
            .get(&pid)
            .is_some_and(|bound| bound != profile_id)
        {
            return;
        }
        if let Some(created) = crate::ipc_server::process_creation_time(pid) {
            if self
                .generations
                .get(&pid)
                .is_some_and(|old| *old != created)
            {
                return;
            }
            self.generations.insert(pid, created);
        }
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
                    dns_config,
                    registry_paths,
                    environment,
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
                    dns_config,
                    registry_paths,
                    environment,
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

    /// Capture the immutable expected bundle before attaching. The host must
    /// retain this bundle while a running process can still depend on it.
    pub fn expect_runtime(&mut self, pid: u32, path: &std::path::Path) -> Result<(), IpcError> {
        let generation = crate::ipc_server::process_creation_time(pid)
            .ok_or_else(|| IpcError::Protocol("cannot query target generation".into()))?;
        if self.generations.get(&pid) != Some(&generation) || !self.bindings.contains_key(&pid) {
            return Err(IpcError::Protocol(
                "target is not bound to this generation".into(),
            ));
        }
        let path = std::fs::canonicalize(path).map_err(|e| IpcError::Io(e.to_string()))?;
        let hash = crate::ipc_server::file_sha256(&path)?;
        if self
            .expected_runtimes
            .get(&pid)
            .is_some_and(|old| old != &(path.clone(), hash.clone()))
        {
            return Err(IpcError::Protocol(
                "Runtime bundle identity conflict".into(),
            ));
        }
        self.expected_runtimes.insert(pid, (path, hash));
        let (path, hash) = self.expected_runtimes[&pid].clone();
        let mut bundle = HashMap::new();
        let architecture = match crate::injection::pe_arch(&path) {
            Ok(crate::injection::PeArch::X64) => "x64",
            Ok(crate::injection::PeArch::X86) => "x86",
            _ => {
                return Err(IpcError::Protocol(
                    "unsupported Runtime bundle architecture".into(),
                ))
            }
        };
        bundle.insert(architecture.to_string(), (path.clone(), hash));
        for (name, architecture, expected_arch) in [
            ("envbox-runtime64.dll", "x64", crate::injection::PeArch::X64),
            ("envbox-runtime32.dll", "x86", crate::injection::PeArch::X86),
        ] {
            let sibling = path.parent().unwrap().join(name);
            if sibling.is_file() && crate::injection::pe_arch(&sibling).ok() == Some(expected_arch)
            {
                let sibling =
                    std::fs::canonicalize(sibling).map_err(|e| IpcError::Io(e.to_string()))?;
                let hash = crate::ipc_server::file_sha256(&sibling)?;
                bundle
                    .entry(architecture.to_string())
                    .or_insert((sibling, hash));
            }
        }
        if self
            .runtime_bundles
            .get(&pid)
            .is_some_and(|old| old != &bundle)
        {
            return Err(IpcError::Protocol(
                "immutable sibling Runtime bundle conflict".into(),
            ));
        }
        self.runtime_bundles.insert(pid, bundle);
        Ok(())
    }

    pub fn runtime_identity(&self, pid: u32) -> Option<&ObservedRuntimeIdentity> {
        if self.generations.get(&pid).copied() != crate::ipc_server::process_creation_time(pid) {
            return None;
        }
        self.identities.get(&pid)
    }

    /// Verify only the declared installed API set. This does not grant whole
    /// process-tree coverage or an application-entry timing guarantee.
    pub fn validate_runtime(&self, pid: u32) -> Result<&ObservedRuntimeIdentity, IpcError> {
        let observed = self
            .runtime_identity(pid)
            .ok_or_else(|| IpcError::Timeout("RUNTIME_IDENTITY".into()))?;
        Self::validate_observation(observed)
    }

    /// Caller must obtain this generation from its original owned process
    /// handle. This reads a previously authenticated observation after exit;
    /// it grants no PID binding, entry approval, or recovery authority.
    pub fn validate_runtime_generation(
        &self,
        pid: u32,
        generation: u64,
    ) -> Result<&ObservedRuntimeIdentity, IpcError> {
        if crate::ipc_server::process_creation_time(pid).is_some_and(|now| now != generation) {
            return Err(IpcError::Protocol("PID generation changed".into()));
        }
        let observed = self
            .identities
            .get(&pid)
            .filter(|id| id.identity.creation_time == generation)
            .or_else(|| self.exited_root_identities.get(&(pid, generation)))
            .ok_or_else(|| IpcError::Timeout("RUNTIME_IDENTITY".into()))?;
        Self::validate_observation(observed)
    }

    fn validate_observation(
        observed: &ObservedRuntimeIdentity,
    ) -> Result<&ObservedRuntimeIdentity, IpcError> {
        let id = &observed.identity;
        if !id.config_complete {
            return Err(IpcError::Protocol(
                "Runtime used incomplete ENV fallback".into(),
            ));
        }
        let config = IpcMessage::decode_line(&id.actual_profile)?;
        let IpcMessage::Profile { dns_mode, .. } = config else {
            return Err(IpcError::Protocol("invalid actual Profile".into()));
        };
        for (group, required) in [
            ("time", 8),
            ("geo", 2),
            ("locale", 14),
            ("language", 6),
            ("registry", 7),
            ("dns", if dns_mode { 15 } else { 2 }),
            // W/A, AsUser and the controlled WithToken refusal are required.
            // An old three-hook bundle cannot claim this child boundary.
            ("process", 4),
            ("network_policy", 1),
        ] {
            let counts: Vec<_> = id.hooks.iter().filter(|(name, _)| name == group).collect();
            let complete = counts.len() == 1
                && (counts[0].1 == required
                    || (group == "dns" && dns_mode && matches!(counts[0].1, 16 | 17)));
            if !complete {
                return Err(IpcError::Protocol(format!(
                    "required hook set incomplete: {group}"
                )));
            }
        }
        Ok(observed)
    }

    /// A fresh Host challenge was answered by this authenticated generation.
    pub fn runtime_reconfirmed(&self, pid: u32) -> bool {
        self.reconfirmed.get(&pid).copied() == crate::ipc_server::process_creation_time(pid)
            && self.reconfirmed.contains_key(&pid)
            && self.validate_runtime(pid).is_ok()
    }

    /// The client acknowledged receiving release outside loader lock. This
    /// proves an EXE entry gate only, not imported-DLL/TLS initialization.
    pub fn startup_gate_released(&self, pid: u32) -> bool {
        self.runtime_identity(pid).is_some_and(|observed| {
            self.events.iter().any(|event| {
                matches!(event, IpcMessage::StartupGateReleased { pid: p, creation_time }
                if *p == pid && *creation_time == observed.identity.creation_time)
            })
        })
    }

    pub(crate) fn handle_client(
        &mut self,
        client: &crate::ipc_server::AuthenticatedProcess,
        msg: &IpcMessage,
    ) -> Option<IpcMessage> {
        use crate::ipc_server::{process_creation_time, process_parent, protocol_denied};
        let denied = |reason: &str| Some(protocol_denied(reason));
        if matches!(
            msg,
            IpcMessage::RegisterProfile { .. } | IpcMessage::BindPid { .. }
        ) {
            return denied("management_command_on_bootstrap_pipe");
        }
        let target = match msg {
            IpcMessage::Hello { pid, .. }
            | IpcMessage::GetProfile { pid, .. }
            | IpcMessage::RuntimeReady { pid }
            | IpcMessage::HookError { pid, .. }
            | IpcMessage::ProcessCreated { pid, .. }
            | IpcMessage::ProcessExited { pid, .. } => *pid,
            IpcMessage::RegisterChild { pid, .. } => *pid,
            IpcMessage::StartupGateReady { pid, .. }
            | IpcMessage::StartupGateReleased { pid, .. }
            | IpcMessage::RuntimeReconnectProof { pid, .. } => *pid,
            IpcMessage::RuntimeIdentity(id)
            | IpcMessage::RuntimeReconnect(id)
            | IpcMessage::RuntimeIdentityConfirm(id) => id.pid,
            _ => return denied("unsupported_bootstrap_message"),
        };
        if target != client.pid && !matches!(msg, IpcMessage::ProcessExited { .. }) {
            return denied("sender_pid_mismatch");
        }
        // Child bootstrap may race the parent's best-effort notice. Resolve
        // inheritance from the OS, with the parent's original generation.
        if !self.bindings.contains_key(&client.pid) {
            if let Some(parent) = process_parent(client.pid) {
                if let (Some(bound_generation), Some(current), Some(profile)) = (
                    self.generations.get(&parent).copied(),
                    process_creation_time(parent),
                    self.bindings.get(&parent).cloned(),
                ) {
                    let inherits = matches!(
                        self.profiles.get(&profile),
                        Some(IpcMessage::Profile {
                            inherit_children: true,
                            ..
                        })
                    );
                    if self.runtime_bundles.contains_key(&parent) {
                        return denied("explicit_child_registration_required");
                    }
                    if inherits && bound_generation == current && current <= client.creation_time {
                        self.bind_pid(client.pid, &profile);
                        self.parents.insert(client.pid, parent);
                    }
                }
            }
        }
        if self.generations.get(&client.pid) != Some(&client.creation_time)
            || !self.bindings.contains_key(&client.pid)
        {
            return denied("unbound_client_generation");
        }
        match msg {
            IpcMessage::RegisterChild {
                creation_time,
                instance_id,
                profile_id,
                child_pid,
                child_creation_time,
                ..
            } => {
                if *creation_time != client.creation_time
                    || self.validate_runtime(client.pid).is_err()
                {
                    return denied("parent_identity_unverified");
                }
                let bound = &self.bindings[&client.pid];
                let Some(IpcMessage::Profile {
                    instance_id: expected_instance,
                    inherit_children: true,
                    ..
                }) = self.profiles.get(bound)
                else {
                    return denied("child_inheritance_disabled");
                };
                if profile_id != bound
                    || instance_id != expected_instance
                    || process_parent(*child_pid) != Some(client.pid)
                    || process_creation_time(*child_pid) != Some(*child_creation_time)
                    || *child_creation_time < client.creation_time
                {
                    return denied("child_or_parent_snapshot_mismatch");
                }
                if self.bindings.get(child_pid).is_some_and(|old| old != bound)
                    || self
                        .generations
                        .get(child_pid)
                        .is_some_and(|old| old != child_creation_time)
                    || self
                        .parents
                        .get(child_pid)
                        .is_some_and(|old| *old != client.pid)
                {
                    return denied("child_binding_conflict");
                }
                let architecture = crate::capability::probe_pid(*child_pid).architecture;
                let Some(bundle) = self.runtime_bundles.get(&client.pid).cloned() else {
                    return denied("parent_runtime_bundle_missing");
                };
                let Some(expected) = bundle.get(architecture).cloned() else {
                    return denied("child_arch_runtime_missing");
                };
                if crate::ipc_server::file_sha256(&expected.0).ok().as_ref() != Some(&expected.1) {
                    return denied("child_runtime_bundle_changed");
                }
                if self
                    .expected_runtimes
                    .get(child_pid)
                    .is_some_and(|old| old != &expected)
                {
                    return denied("child_runtime_identity_conflict");
                }
                let profile_id = bound.clone();
                self.bind_pid(*child_pid, &profile_id);
                self.parents.insert(*child_pid, client.pid);
                self.expected_runtimes.insert(*child_pid, expected);
                self.runtime_bundles.insert(*child_pid, bundle);
                if !self.events.contains(msg) {
                    self.events.push(msg.clone());
                }
                Some(IpcMessage::ChildBound {
                    pid: *child_pid,
                    creation_time: *child_creation_time,
                })
            }
            IpcMessage::StartupGateReady { pid, creation_time } => {
                if *creation_time != client.creation_time || self.validate_runtime(*pid).is_err() {
                    return denied("startup_gate_identity_or_capability_mismatch");
                }
                self.events.push(msg.clone());
                Some(IpcMessage::StartupRelease {
                    pid: *pid,
                    creation_time: *creation_time,
                })
            }
            IpcMessage::StartupGateReleased { pid, creation_time } => {
                if *creation_time != client.creation_time
                    || self.validate_runtime(*pid).is_err()
                    || !self.events.iter().any(|event| {
                        matches!(event,
                        IpcMessage::StartupGateReady { pid: p, creation_time: c }
                        if p == pid && c == creation_time)
                    })
                {
                    return denied("startup_gate_release_without_approval");
                }
                self.events.push(msg.clone());
                Some(IpcMessage::StartupGateConfirmed {
                    pid: *pid,
                    creation_time: *creation_time,
                })
            }
            IpcMessage::GetProfile { profile_id, .. } => {
                let bound = &self.bindings[&client.pid];
                if !profile_id.is_empty() && profile_id != bound {
                    return denied("profile_hint_mismatch");
                }
                match self.profiles.get(bound) {
                    Some(profile) if profile.encode_line().len() < IPC_MAX_LINE_BYTES => {
                        Some(profile.clone())
                    }
                    Some(_) => denied("profile_exceeds_wire_limit"),
                    None => denied("profile_missing"),
                }
            }
            IpcMessage::Hello { instance_id, .. } => {
                let Some(IpcMessage::Profile {
                    instance_id: expected,
                    ..
                }) = self.profiles.get(&self.bindings[&client.pid])
                else {
                    return denied("profile_missing");
                };
                if !instance_id.is_empty() && instance_id != expected {
                    return denied("instance_hint_mismatch");
                }
                self.events.push(msg.clone());
                None
            }
            IpcMessage::ProcessCreated { child_pid, .. } => {
                let inherits = matches!(
                    self.profiles.get(&self.bindings[&client.pid]),
                    Some(IpcMessage::Profile {
                        inherit_children: true,
                        ..
                    })
                );
                let child_created = process_creation_time(*child_pid);
                if !inherits
                    || process_parent(*child_pid) != Some(client.pid)
                    || child_created.is_none_or(|created| created < client.creation_time)
                {
                    return denied("child_identity_mismatch");
                }
                self.handle(msg)
            }
            IpcMessage::ProcessExited { pid, .. } => {
                if *pid != client.pid && self.parents.get(pid) != Some(&client.pid) {
                    return denied("exit_membership_mismatch");
                }
                if let Some(current) = process_creation_time(*pid) {
                    if self.generations.get(pid) != Some(&current) {
                        return denied("exit_generation_mismatch");
                    }
                }
                self.handle(msg)
            }
            IpcMessage::RuntimeReconnectProof {
                pid,
                creation_time,
                nonce,
            } => {
                if *creation_time != client.creation_time
                    || self.validate_runtime(*pid).is_err()
                    || self.reconnect_challenges.remove(pid).as_ref() != Some(nonce)
                {
                    return denied("reconnect_challenge_mismatch");
                }
                self.reconfirmed.insert(*pid, *creation_time);
                self.events.push(msg.clone());
                Some(IpcMessage::RuntimeReconnected {
                    pid: *pid,
                    creation_time: *creation_time,
                })
            }
            IpcMessage::RuntimeIdentity(id)
            | IpcMessage::RuntimeReconnect(id)
            | IpcMessage::RuntimeIdentityConfirm(id) => {
                if id.creation_time != client.creation_time
                    || id.protocol != RUNTIME_IDENTITY_PROTOCOL
                    || id.runtime_version != env!("CARGO_PKG_VERSION")
                {
                    return denied("runtime_protocol_or_generation_mismatch");
                }
                let Ok(actual) = IpcMessage::decode_line(&id.actual_profile) else {
                    return denied("invalid_actual_profile");
                };
                let expected = self.profiles.get(&self.bindings[&client.pid]);
                if expected != Some(&actual) {
                    return denied("actual_profile_mismatch");
                }
                let Some((path, hash)) = self.expected_runtimes.get(&client.pid) else {
                    return denied("expected_runtime_missing");
                };
                let actual_path = std::fs::canonicalize(&id.module_path).ok();
                if actual_path.as_ref() != Some(path)
                    || !crate::ipc_server::process_has_module(client.pid, path)
                    || crate::ipc_server::file_sha256(path).ok().as_ref() != Some(hash)
                {
                    return denied("actual_runtime_module_mismatch");
                }
                if self
                    .identities
                    .get(&client.pid)
                    .is_some_and(|old| &old.identity != id)
                {
                    return denied("immutable_runtime_identity_conflict");
                }
                use sha2::{Digest, Sha256};
                let config_sha256 =
                    format!("{:x}", Sha256::digest(actual.encode_line().as_bytes()));
                self.identities.insert(
                    client.pid,
                    ObservedRuntimeIdentity {
                        identity: id.clone(),
                        module_sha256: hash.clone(),
                        config_sha256,
                    },
                );
                self.events.push(msg.clone());
                if matches!(msg, IpcMessage::RuntimeReconnect(_)) {
                    if self.validate_runtime(client.pid).is_err() {
                        self.identities.remove(&client.pid);
                        return denied("reconnect_capability_incomplete");
                    }
                    self.reconfirmed.remove(&client.pid);
                    let nonce = uuid::Uuid::new_v4().to_string();
                    self.reconnect_challenges.insert(client.pid, nonce.clone());
                    Some(IpcMessage::RuntimeReconnectChallenge {
                        pid: client.pid,
                        creation_time: client.creation_time,
                        nonce,
                    })
                } else if matches!(msg, IpcMessage::RuntimeIdentityConfirm(_)) {
                    if self.validate_runtime(client.pid).is_err() {
                        self.identities.remove(&client.pid);
                        return denied("identity_capability_incomplete");
                    }
                    Some(IpcMessage::RuntimeIdentityConfirmed {
                        pid: client.pid,
                        creation_time: client.creation_time,
                    })
                } else {
                    None
                }
            }
            _ => self.handle(msg),
        }
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
                        dns_config: None,
                        registry_paths: vec![],
                        environment: vec![],
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
                if !self.parents.contains_key(pid) {
                    if let Some(identity) = self.identities.get(pid) {
                        self.exited_root_identities
                            .insert((*pid, identity.identity.creation_time), identity.clone());
                    }
                }
                self.generations.remove(pid);
                self.identities.remove(pid);
                self.reconnect_challenges.remove(pid);
                self.reconfirmed.remove(pid);
                self.expected_runtimes.remove(pid);
                self.runtime_bundles.remove(pid);
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
                dns_config: None,
                registry_paths: vec!["HKCU\\Software\\EnvBox".into()],
                environment: vec![("LANG".into(), "en_US.UTF-8".into())],
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
                ..Default::default()
            },
            environment: HashMap::from([
                ("LANG".into(), "en_US.UTF-8".into()),
                ("TOOL_OPTIONS".into(), "alpha beta=gamma".into()),
            ]),
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
        assert_eq!(decoded.environment, profile.environment);
        assert_eq!(
            decoded.registry.whitelist_paths,
            profile.registry.whitelist_paths
        );
        assert_eq!(decoded.browser.webrtc, profile.browser.webrtc);
    }

    #[test]
    fn runtime_without_with_token_boundary_cannot_be_accepted() {
        let mut observed = ObservedRuntimeIdentity {
            identity: RuntimeIdentity {
                pid: 1,
                creation_time: 1,
                protocol: RUNTIME_IDENTITY_PROTOCOL,
                runtime_version: env!("CARGO_PKG_VERSION").into(),
                module_path: "fixture-runtime.dll".into(),
                actual_profile: "PROFILE profile_id=p instance_id=i locale_name=en-US ui_language=en-US region=US tz_windows=UTC tz_iana=Etc/UTC inherit_children=1 audit=0 webrtc=host dns_mode=0".into(),
                config_complete: true,
                hooks: [
                    ("time", 8), ("geo", 2), ("locale", 14), ("language", 6),
                    ("registry", 7), ("dns", 2), ("process", 3),
                    ("network_policy", 1),
                ].into_iter().map(|(name, count)| (name.into(), count)).collect(),
            },
            module_sha256: "fixture".into(),
            config_sha256: "fixture".into(),
        };
        let rejected = SessionTable::validate_observation(&observed).unwrap_err();
        assert!(rejected
            .to_string()
            .contains("required hook set incomplete: process"));
        observed
            .identity
            .hooks
            .iter_mut()
            .find(|(name, _)| name == "process")
            .unwrap()
            .1 = 4;
        assert!(SessionTable::validate_observation(&observed).is_ok());
        // Duplicating a count does not establish another attached API.
        observed.identity.hooks.push(("process".into(), 4));
        assert!(SessionTable::validate_observation(&observed).is_err());
    }

    #[test]
    fn doh_ipc_preserves_policy_and_rejects_missing_or_unknown_values() {
        let mut profile = EnvironmentProfile {
            id: Uuid::nil(),
            name: "DoH".into(),
            locale: LocaleProfile {
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
            },
            timezone: TimezoneProfile {
                windows_id: "Pacific Standard Time".into(),
                iana_id: "America/Los_Angeles".into(),
            },
            dns: Default::default(),
            environment: Default::default(),
            registry: Default::default(),
            browser: Default::default(),
        };
        profile.dns = envbox_core::DnsProfile::typed(
            envbox_core::DnsMode::VirtualView,
            true,
            vec![envbox_core::DnsUpstream::Doh {
                url: "https://1.1.1.1/dns-query".into(),
                bootstrap_ips: vec![],
                tls_revocation: envbox_core::DnsTlsRevocation::StrictOffline,
            }],
        );
        let line = profile_to_message(&profile, "instance").encode_line();
        assert!(line.contains("dns_upstream_0_tls_revocation=1"));
        let decoded = IpcMessage::decode_line(&line).unwrap();
        assert_eq!(message_to_profile(&decoded).unwrap().dns, profile.dns);
        assert!(
            IpcMessage::decode_line(&line.replace(" dns_upstream_0_tls_revocation=1", "")).is_err()
        );
        assert!(IpcMessage::decode_line(&line.replace(
            "dns_upstream_0_tls_revocation=1",
            "dns_upstream_0_tls_revocation=2"
        ))
        .is_err());
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
                ..Default::default()
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
                ..Default::default()
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
            dns_config: None,
            registry_paths: vec![],
            environment: vec![],
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
            dns_config: None,
            registry_paths: vec![],
            environment: vec![("LANG".into(), "en_US.UTF-8".into())],
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
            dns_config: None,
            registry_paths: vec![],
            environment: vec![("LANG".into(), "en_US.UTF-8".into())],
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
