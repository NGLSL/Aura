//! Stable line protocol: message types, bounded decoding, encoding and escaping.
use envbox_core::DnsMode;
use std::collections::HashMap;
use thiserror::Error;

/// Default pipe name. Override with `ENVBOX_IPC_PIPE`.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\envbox-runtime";
pub const IPC_MAX_LINE_BYTES: usize = 8192;
pub const IPC_IDENTITY_MAX_LINE_BYTES: usize = 32768;
pub const RUNTIME_IDENTITY_PROTOCOL: u32 = 1;

/// Facts observed after the Runtime's hook transaction committed. Counts
/// describe attached APIs, never complete Windows/process-tree coverage.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
        identity: envbox_core::IdentityProfile,
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
        identity: envbox_core::IdentityProfile,
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
                identity,
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
                for (key, value) in identity.flat_fields() { s.push_str(&format!(" {key}={}", quote_value(&value))); }
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
                identity,
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
                for (key, value) in identity.flat_fields() { s.push_str(&format!(" {key}={}", quote_value(&value))); }
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
                identity: decode_identity_fields(&map)?,
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
                identity: decode_identity_fields(&map)?,
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

fn decode_identity_fields(
    fields: &[(String, String)],
) -> Result<envbox_core::IdentityProfile, IpcError> {
    let mut identity = envbox_core::IdentityProfile::default();
    for (key, value) in fields
        .iter()
        .filter(|(key, _)| key.starts_with("identity_"))
    {
        let target = match key.as_str() {
            "identity_computer_name" => &mut identity.computer_name,
            "identity_user_name" => &mut identity.user_name,
            "identity_mac_address" => &mut identity.mac_address,
            "identity_machine_guid" => &mut identity.machine_guid,
            _ => return Err(IpcError::Protocol("unknown identity field".into())),
        };
        if target.replace(value.clone()).is_some() {
            return Err(IpcError::Protocol("duplicate identity field".into()));
        }
    }
    identity
        .validate()
        .map_err(|err| IpcError::Protocol(err.to_string()))?;
    for (_, entry) in fields.iter().filter(|(key, _)| key == "environment") {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| IpcError::Protocol("invalid environment entry".into()))?;
        if key.to_ascii_uppercase().starts_with("ENVBOX_IDENTITY_") {
            return Err(IpcError::Protocol(
                "reserved identity environment variable".into(),
            ));
        }
        for (name, expected) in [
            ("COMPUTERNAME", identity.computer_name.as_ref()),
            ("USERNAME", identity.user_name.as_ref()),
        ] {
            if key.eq_ignore_ascii_case(name) && expected.is_some_and(|expected| expected != value)
            {
                return Err(IpcError::Protocol("identity environment conflict".into()));
            }
        }
    }
    Ok(identity)
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
