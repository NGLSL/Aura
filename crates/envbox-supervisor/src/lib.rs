//! Independent, authenticated management channel. Runtime bootstrap uses a
//! different pipe and cannot grant management authority.
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub generation: Option<String>,
    pub request_id: String,
    pub command: String,
    #[serde(default)]
    pub run: Option<RunCommand>,
    #[serde(default)]
    pub container_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCommand {
    pub container_id: uuid::Uuid,
    pub instance_id: uuid::Uuid,
    pub application_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub record_schema: u32,
    pub request_id: String,
    pub supervisor_generation: String,
    pub job_name: String,
    pub snapshot_digest: String,
    pub container_id: uuid::Uuid,
    pub instance_id: uuid::Uuid,
    pub application_id: uuid::Uuid,
    pub root_pid: u32,
    pub profile_id: uuid::Uuid,
    #[serde(default)]
    pub runtime_module_path: PathBuf,
    #[serde(default)]
    pub runtime_module_sha256: String,
    #[serde(default)]
    pub runtime_config_sha256: String,
    #[serde(default)]
    pub runtime_version: String,
    /// Last authenticated root observation; absent on legacy/unobserved records.
    #[serde(default)]
    pub environment_facts: Option<EnvironmentRuntimeFacts>,
    #[serde(default)]
    pub audit: bool,
    #[serde(default)]
    pub inherit_children: bool,
    #[serde(default)]
    pub known_members: Vec<ProcessIdentity>,
    #[serde(default)]
    pub member_runtimes: Vec<MemberRuntimeIdentity>,
    pub creation_time: u64,
    pub mode: String,
    pub entry_guarantee: String,
    pub storage_policy_enforced: bool,
    pub configuration_id: String,
    pub error: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_time: u64,
}

/// Facts authenticated by the instance broker for one actual Job member.
/// These are sealed before recovery can reuse that member's Runtime bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRuntimeIdentity {
    pub pid: u32,
    pub creation_time: u64,
    pub module_path: PathBuf,
    pub module_sha256: String,
    pub config_sha256: String,
    pub runtime_version: String,
    #[serde(default)]
    pub environment_facts: Option<EnvironmentRuntimeFacts>,
}

/// Installation facts, not proof of API semantics or complete process coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentRuntimeFacts {
    pub config_complete: bool,
    pub profile_matches_snapshot: bool,
    pub hooks: Vec<InstalledHookFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledHookFact {
    pub group: String,
    pub attached_api_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunView {
    pub result: RunResult,
    pub process_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub generation: String,
    pub request_id: String,
    pub supervisor_pid: u32,
    pub status: String,
    #[serde(default)]
    pub run: Option<RunResult>,
    #[serde(default)]
    pub instances: Vec<RunView>,
}

/// A file identity approved by the server owner before accepting connections.
/// These are never accepted from a management request or runtime registration.
#[derive(Debug, Clone)]
pub struct ApprovedManager {
    pub path: PathBuf,
    pub sha256: String,
}

impl ApprovedManager {
    pub fn from_file(path: &Path) -> io::Result<Self> {
        Ok(Self {
            path: std::fs::canonicalize(path)?,
            sha256: file_hash(path)?,
        })
    }
}

fn file_hash(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(windows)]
mod records;
#[cfg(windows)]
mod runs;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{run_default, serve, ManagedTarget, ServerConfig, SupervisorClient};

#[cfg(not(windows))]
pub fn run_default() -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Windows only"))
}

/// Default upper bound for startup and each management exchange.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
