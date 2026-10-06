use envbox_core::RunSnapshot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

pub const MAX_FRAME: usize = 65_536;
pub const SERVICE_NAME: &str = "AuraPolicyService";
pub const PIPE_NAME: &str = r"\\.\pipe\AuraPolicyService.v1";
pub const SERVICE_SID: [u8; 32] = [
    1, 6, 0, 0, 0, 0, 0, 5, 0x50, 0, 0, 0, 0xce, 0x9d, 0xb8, 0xe3, 0x14, 0xcd, 0x7c, 0x67, 0x99,
    0x2a, 0xa2, 0x5c, 0xc4, 0x0c, 0x19, 0x7c, 0x4e, 0x2a, 0x18, 0x1c,
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NetworkPolicy {
    Host,
    Deny,
}

/// The request carries no PID, token, kernel handle, SID or capability flag.
/// Identity and token are independently derived from the connected pipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRequest {
    pub version: u32,
    pub request_id: Uuid,
    pub instance_id: Uuid,
    pub container_id: Uuid,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub profile_id: Uuid,
    pub snapshot_id: Uuid,
    pub configuration_id: String,
    pub content_digest: String,
    pub network: NetworkPolicy,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum Command {
    Launch { request: Box<LaunchRequest> },
    Status { instance_id: Uuid },
    Stop { instance_id: Uuid },
}
pub fn decode_command(bytes: &[u8]) -> Result<Command, String> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("invalid frame length".into());
    }
    let command: Command = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    match &command {
        Command::Launch { request } => {
            decode(&serde_json::to_vec(request).map_err(|e| e.to_string())?)?;
        }
        Command::Status { instance_id } | Command::Stop { instance_id } => {
            if instance_id.is_nil() {
                return Err("nil run identity".into());
            }
        }
    }
    Ok(command)
}

pub fn decode(bytes: &[u8]) -> Result<LaunchRequest, String> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("invalid frame length".into());
    }
    let r: LaunchRequest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    fn has_nul(v: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::String(s) => s.contains('\0'),
            serde_json::Value::Array(a) => a.iter().any(has_nul),
            serde_json::Value::Object(o) => o.iter().any(|(k, v)| k.contains('\0') || has_nul(v)),
            _ => false,
        }
    }
    if has_nul(&serde_json::to_value(&r).map_err(|e| e.to_string())?) {
        return Err("NUL in request".into());
    }
    if r.version != 1
        || r.request_id.is_nil()
        || r.instance_id.is_nil()
        || r.container_id.is_nil()
        || r.profile_id.is_nil()
        || r.snapshot_id != r.instance_id
    {
        return Err("invalid version or identity".into());
    }
    if [&r.configuration_id, &r.content_digest]
        .iter()
        .any(|s| s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("invalid snapshot digest reference".into());
    }
    for path in [&r.executable, &r.working_directory] {
        validate_local_path(path)?;
    }
    if r.arguments.len() > 256 || r.arguments.iter().any(|a| a.contains('\0')) {
        return Err("invalid arguments".into());
    }
    if r.executable.to_string_lossy().contains('\0')
        || r.working_directory.to_string_lossy().contains('\0')
    {
        return Err("NUL in path".into());
    }
    Ok(r)
}
pub(crate) fn validate_local_path(path: &std::path::Path) -> Result<(), String> {
    envbox_launcher::service_start::validate_service_input_path(path)
}

pub fn policy_wire(
    generation: u64,
    process_handle: u64,
    r: &LaunchRequest,
    snapshot: &RunSnapshot,
) -> Result<[u8; 136], String> {
    if generation == 0 || process_handle == 0 || process_handle == u64::MAX {
        return Err("invalid driver generation or handle".into());
    }
    crate::snapshot::validate_reference(r, snapshot)?;
    let canonical = serde_json::to_value((r, snapshot)).map_err(|e| e.to_string())?;
    let digest = Sha256::digest(serde_json::to_vec(&canonical).map_err(|e| e.to_string())?);
    let mut b = [0u8; 136];
    for (offset, value) in [
        (0, 1u32),
        (4, 136),
        (8, 1),
        (12, u32::from(r.network == NetworkPolicy::Deny)),
    ] {
        b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    b[16..24].copy_from_slice(&generation.to_le_bytes());
    b[24..32].copy_from_slice(&process_handle.to_le_bytes());
    b[32..64].copy_from_slice(&SERVICE_SID);
    b[64..80].copy_from_slice(r.container_id.as_bytes());
    b[80..96].copy_from_slice(r.instance_id.as_bytes());
    b[96..128].copy_from_slice(&digest);
    Ok(b)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> (LaunchRequest, RunSnapshot, envbox_core::Container) {
        let profile = envbox_core::EnvironmentProfile {
            id: Uuid::new_v4(),
            name: "test".into(),
            locale: envbox_core::LocaleProfile {
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
            },
            timezone: envbox_core::TimezoneProfile {
                windows_id: "UTC".into(),
                iana_id: "Etc/UTC".into(),
            },
            dns: envbox_core::DnsProfile::default(),
            environment: Default::default(),
            registry: Default::default(),
            browser: Default::default(),
        };
        let container = envbox_core::Container::new("service test", profile.id);
        let instance = Uuid::new_v4();
        let snapshot = RunSnapshot::new(&container, &profile, instance).unwrap();
        let r = LaunchRequest {
            version: 1,
            request_id: Uuid::new_v4(),
            instance_id: instance,
            container_id: container.id,
            profile_id: profile.id,
            snapshot_id: instance,
            configuration_id: snapshot.configuration_id.clone(),
            content_digest: snapshot.content_digest.clone(),
            executable: PathBuf::from(r"C:\Windows\System32\notepad.exe"),
            arguments: vec![],
            working_directory: PathBuf::from(r"C:\Windows"),
            network: NetworkPolicy::Deny,
        };
        (r, snapshot, container)
    }
    #[test]
    fn frame_bounds_and_foreign_fields_fail() {
        assert!(decode(&[]).is_err());
        assert!(decode(&vec![0; MAX_FRAME + 1]).is_err());
        let (r, _, _) = fixture();
        let mut value = serde_json::to_value(r).unwrap();
        value["profile"] = serde_json::json!({"locale":"fake"});
        assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn service_sid_matches_kernel_identity() {
        assert_eq!(&SERVICE_SID[..12], &[1, 6, 0, 0, 0, 0, 0, 5, 80, 0, 0, 0]);
        assert_eq!(
            u32::from_le_bytes(SERVICE_SID[12..16].try_into().unwrap()),
            3820527054
        );
    }
    #[test]
    fn wire_matches_offsets_and_requires_authoritative_snapshot() {
        let (mut r, s, _) = fixture();
        let b = policy_wire(17, 0x1234, &r, &s).unwrap();
        assert_eq!(
            &b[0..16],
            &[1, 0, 0, 0, 136, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]
        );
        assert_eq!(&b[16..24], &17u64.to_le_bytes());
        assert_eq!(&b[24..32], &0x1234u64.to_le_bytes());
        assert_eq!(&b[32..64], &SERVICE_SID);
        assert_eq!(&b[64..80], r.container_id.as_bytes());
        assert_ne!(&b[64..80], r.profile_id.as_bytes());
        assert_eq!(&b[80..96], r.instance_id.as_bytes());
        assert_eq!(&b[128..], &[0; 8]);
        assert!(policy_wire(0, 42, &r, &s).is_err());
        assert!(policy_wire(1, u64::MAX, &r, &s).is_err());
        r.content_digest = "0".repeat(64);
        assert!(policy_wire(1, 42, &r, &s).is_err());
    }
    #[test]
    fn identity_and_digest_references_fail_closed() {
        let (mut r, _, _) = fixture();
        r.snapshot_id = Uuid::nil();
        assert!(decode(&serde_json::to_vec(&r).unwrap()).is_err());
        let (mut r, _, _) = fixture();
        r.configuration_id = "wrong".into();
        assert!(decode(&serde_json::to_vec(&r).unwrap()).is_err());
    }
    #[test]
    fn envelope_digest_binds_network_policy() {
        let (mut r, s, _) = fixture();
        let b = policy_wire(1, 42, &r, &s).unwrap();
        r.network = NetworkPolicy::Host;
        assert_ne!(&b[96..128], &policy_wire(1, 42, &r, &s).unwrap()[96..128]);
    }
    #[test]
    fn commands_reject_foreign_fields_and_nil_run() {
        assert!(decode_command(
            br#"{"operation":"Stop","instance_id":"00000000-0000-0000-0000-000000000000"}"#
        )
        .is_err());
        let (r, _, _) = fixture();
        assert!(decode_command(
            &serde_json::to_vec(&Command::Launch {
                request: Box::new(r)
            })
            .unwrap()
        )
        .is_ok());
    }
    #[test]
    fn paths_fail_before_system_open() {
        for path in [
            r"\\server\share\x.exe",
            r"\\?\C:\x.exe",
            r"C:\dir\..\x.exe",
            r"C:\x.exe:stream",
            "C:\\x\0y",
            r"C:\dir\.\file.exe",
            r"C:\dir.\file.exe",
            r"C:\dir \file.exe",
            r"C:\dir\NUL.exe",
        ] {
            assert!(
                validate_local_path(std::path::Path::new(path)).is_err(),
                "{path}"
            );
            assert!(crate::bundle::InstalledBundle::lease_input_path(
                std::path::Path::new(path),
                false
            )
            .is_err());
        }
    }
}
