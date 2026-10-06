//! Immutable prepared configuration. Preparing it does not start a process or authorize IPC.
use crate::storage_policy::StoragePolicy;
use crate::{Container, ContainerMode, DomainError, EnvironmentProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MAX_RUN_SNAPSHOT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "SnapshotWire")]
pub struct RunSnapshot {
    pub schema_version: u32,
    pub profile_schema_version: u32,
    pub snapshot_id: Uuid,
    pub instance_id: Uuid,
    pub container_id: Uuid,
    pub created_at_unix_ms: u64,
    pub configuration_id: String,
    pub content_digest: String,
    pub mode: ContainerMode,
    pub effective_profile: EnvironmentProfile,
    pub storage_policy: StoragePolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    schema_version: u32,
    profile_schema_version: u32,
    snapshot_id: Uuid,
    instance_id: Uuid,
    container_id: Uuid,
    created_at_unix_ms: u64,
    configuration_id: String,
    content_digest: String,
    mode: ContainerMode,
    effective_profile: serde_json::Value,
    storage_policy: StoragePolicy,
}
impl TryFrom<SnapshotWire> for RunSnapshot {
    type Error = String;
    fn try_from(wire: SnapshotWire) -> Result<Self, Self::Error> {
        let profile =
            decode_effective_profile(wire.effective_profile, wire.profile_schema_version)?;
        let snapshot = Self {
            schema_version: wire.schema_version,
            profile_schema_version: wire.profile_schema_version,
            snapshot_id: wire.snapshot_id,
            instance_id: wire.instance_id,
            container_id: wire.container_id,
            created_at_unix_ms: wire.created_at_unix_ms,
            configuration_id: wire.configuration_id,
            content_digest: wire.content_digest,
            mode: wire.mode,
            effective_profile: profile,
            storage_policy: wire.storage_policy,
        };
        snapshot.validate().map_err(|err| err.to_string())?;
        Ok(snapshot)
    }
}
impl Serialize for RunSnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.wire_value()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}
fn decode_effective_profile(
    value: serde_json::Value,
    version: u32,
) -> Result<EnvironmentProfile, String> {
    fn fields(value: &serde_json::Value, required: &[&str]) -> Result<(), String> {
        let table = value
            .as_object()
            .ok_or("expected complete effective Profile table")?;
        if table.len() != required.len() || required.iter().any(|key| !table.contains_key(*key)) {
            return Err("missing or unknown effective Profile field".into());
        }
        Ok(())
    }
    fields(
        &value,
        &[
            "id",
            "name",
            "locale",
            "timezone",
            "dns",
            "environment",
            "registry",
            "browser",
        ],
    )?;
    let dns_fields = match version {
        1 => &["mode", "servers"][..],
        2 => &["mode", "strict", "upstreams"][..],
        _ => return Err("unsupported Profile snapshot schema".into()),
    };
    fields(&value["dns"], dns_fields)?;
    if version == 2 {
        let upstreams = value["dns"]["upstreams"]
            .as_array()
            .ok_or("expected complete DNS upstream array")?;
        for upstream in upstreams {
            let required = match upstream.get("type").and_then(serde_json::Value::as_str) {
                Some("udp" | "tcp") => &["type", "address", "port"][..],
                Some("dot") => &["type", "address", "port", "server_name"][..],
                Some("doh") => &["type", "url", "bootstrap_ips", "tls_revocation"][..],
                _ => return Err("unknown snapshot DNS transport".into()),
            };
            fields(upstream, required)?;
        }
    }
    for (key, required) in [
        ("locale", &["locale_name", "ui_language", "region"][..]),
        ("timezone", &["windows_id", "iana_id"][..]),
        ("registry", &["whitelist_paths"][..]),
        ("browser", &["webrtc"][..]),
    ] {
        fields(&value[key], required)?;
    }
    serde_json::from_value(value).map_err(|err| err.to_string())
}
fn profile_wire_value(
    profile: &EnvironmentProfile,
    version: u32,
) -> Result<serde_json::Value, DomainError> {
    let mut value =
        serde_json::to_value(profile).map_err(|_| invalid("Profile serialization failed"))?;
    if version == 1 {
        if !profile.dns.strict {
            return Err(invalid("legacy snapshot cannot represent non-strict DNS"));
        }
        let servers: Option<Vec<std::net::IpAddr>> = profile
            .dns
            .effective_upstreams()
            .iter()
            .map(|upstream| match upstream {
                crate::DnsUpstream::Udp { address, port: 53 } => Some(*address),
                _ => None,
            })
            .collect();
        let servers =
            servers.ok_or_else(|| invalid("legacy snapshot cannot represent typed transport"))?;
        value["dns"] = serde_json::json!({"mode": profile.dns.mode, "servers": servers});
    }
    Ok(value)
}

impl RunSnapshot {
    pub fn new(
        container: &Container,
        profile: &EnvironmentProfile,
        instance_id: Uuid,
    ) -> Result<Self, DomainError> {
        container.validate()?;
        profile.validate()?;
        if profile.id != container.profile_id || instance_id.is_nil() {
            return Err(invalid("invalid Instance identity or Profile binding"));
        }
        let configuration_id =
            configuration_digest(container.mode, profile, &container.storage_policy, 2)?;
        let mut snapshot = Self {
            schema_version: 1,
            profile_schema_version: 2,
            snapshot_id: instance_id,
            instance_id,
            container_id: container.id,
            created_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| invalid("clock predates Unix epoch"))?
                .as_millis() as u64,
            configuration_id,
            content_digest: String::new(),
            mode: container.mode,
            effective_profile: profile.clone(),
            storage_policy: container.storage_policy.clone(),
        };
        snapshot.content_digest = snapshot.digest()?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != 1
            || !matches!(self.profile_schema_version, 1 | 2)
            || self.storage_policy.schema_version != 1
        {
            return Err(invalid("unsupported snapshot/Profile/storage schema"));
        }
        if self.container_id.is_nil()
            || self.instance_id.is_nil()
            || self.snapshot_id != self.instance_id
            || self.mode != ContainerMode::Compatibility
        {
            return Err(invalid("invalid snapshot identity or unsupported mode"));
        }
        self.effective_profile.validate()?;
        if self
            .effective_profile
            .environment
            .values()
            .any(|value| value.contains('\0'))
        {
            return Err(invalid("NUL in effective environment value"));
        }
        if self.configuration_id
            != configuration_digest(
                self.mode,
                &self.effective_profile,
                &self.storage_policy,
                self.profile_schema_version,
            )?
            || self.content_digest != self.digest()?
        {
            return Err(invalid("configuration or content digest mismatch"));
        }
        if canonical_json(self)?.len() > MAX_RUN_SNAPSHOT_BYTES {
            return Err(invalid("snapshot exceeds one MiB limit"));
        }
        Ok(())
    }

    fn wire_value(&self) -> Result<serde_json::Value, DomainError> {
        Ok(serde_json::json!({
            "schema_version": self.schema_version, "profile_schema_version": self.profile_schema_version,
            "snapshot_id": self.snapshot_id, "instance_id": self.instance_id, "container_id": self.container_id,
            "created_at_unix_ms": self.created_at_unix_ms, "configuration_id": self.configuration_id,
            "content_digest": self.content_digest, "mode": self.mode,
            "effective_profile": profile_wire_value(&self.effective_profile, self.profile_schema_version)?,
            "storage_policy": self.storage_policy,
        }))
    }
    fn digest(&self) -> Result<String, DomainError> {
        let mut content = self.clone();
        content.content_digest.clear();
        digest(&content)
    }
}

fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidContainer(format!("Run snapshot: {reason}"))
}
fn configuration_digest(
    mode: ContainerMode,
    profile: &EnvironmentProfile,
    policy: &StoragePolicy,
    version: u32,
) -> Result<String, DomainError> {
    digest(&(version, mode, profile_wire_value(profile, version)?, policy))
}
fn digest(value: &impl Serialize) -> Result<String, DomainError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(canonical_json(value)?.as_bytes())
    ))
}

// Explicitly sort map keys, including environment entries, regardless of serde_json features.
fn canonical_json(value: &impl Serialize) -> Result<String, DomainError> {
    let value = serde_json::to_value(value).map_err(|_| invalid("serialization failed"))?;
    fn write(value: &serde_json::Value, output: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                output.push('{');
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                for (index, key) in keys.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(key).expect("JSON string"));
                    output.push(':');
                    write(&map[*key], output);
                }
                output.push('}');
            }
            serde_json::Value::Array(items) => {
                output.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    write(item, output);
                }
                output.push(']');
            }
            other => output.push_str(&other.to_string()),
        }
    }
    let mut output = String::new();
    write(&value, &mut output);
    Ok(output)
}
