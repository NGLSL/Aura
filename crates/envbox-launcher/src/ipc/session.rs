//! Session membership, authenticated Runtime observations and lifecycle state.
use super::profile::{profile_to_message, profile_to_message_with_flags};
use super::wire::{
    IpcError, IpcMessage, RuntimeIdentity, IPC_MAX_LINE_BYTES, RUNTIME_IDENTITY_PROTOCOL,
};
use envbox_core::EnvironmentProfile;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedRuntimeIdentity {
    pub identity: RuntimeIdentity,
    pub module_sha256: String,
    pub config_sha256: String,
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
                    identity,
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
                    identity,
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

    pub(super) fn validate_observation(
        observed: &ObservedRuntimeIdentity,
    ) -> Result<&ObservedRuntimeIdentity, IpcError> {
        let id = &observed.identity;
        if !id.config_complete {
            return Err(IpcError::Protocol(
                "Runtime used incomplete ENV fallback".into(),
            ));
        }
        let config = IpcMessage::decode_line(&id.actual_profile)?;
        let IpcMessage::Profile {
            dns_mode,
            ref identity,
            ..
        } = config
        else {
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
        if !identity.is_host() {
            let required = 6 * u32::from(identity.computer_name.is_some())
                + 2 * u32::from(identity.user_name.is_some())
                + 5 * u32::from(identity.mac_address.is_some());
            let counts: Vec<_> = id
                .hooks
                .iter()
                .filter(|(name, _)| name == "identity")
                .collect();
            if counts.len() != 1 || counts[0].1 != required {
                return Err(IpcError::Protocol(
                    "required hook set incomplete: identity".into(),
                ));
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
                        identity: Default::default(),
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
