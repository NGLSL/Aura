//! Source qualification seam, deliberately disconnected from production SCM
//! request dispatch. Creating this engine requires actual service identity,
//! protected installed bundle leases and a live driver session. It cannot
//! elevate the core Container capability and is not a deployment switch.
use crate::{
    bundle::InstalledBundle,
    protocol::LaunchRequest,
    windows_service::{auth_id, user, AuthenticatedClient, DriverSession},
};
use envbox_launcher::{
    service_start::{start_session_as_user, ServiceStartOptions},
    SessionHandle, SessionStartRequest,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

type Result<T> = std::result::Result<T, String>;
#[derive(Clone, PartialEq, Eq)]
struct Owner {
    sid: Vec<u8>,
    auth: (u32, i32),
    session: u32,
}
struct ManagedRun {
    owner: Owner,
    request: LaunchRequest,
    session: SessionHandle,
    control_error: Option<String>,
}
struct RequestRecord {
    owner: Owner,
    digest: [u8; 32],
    outcome: Result<Uuid>,
}
pub struct PrototypeEngine {
    bundle: InstalledBundle,
    driver: DriverSession,
    runs: HashMap<Uuid, ManagedRun>,
    requests: HashMap<Uuid, RequestRecord>,
}
fn owner(client: &AuthenticatedClient) -> Result<Owner> {
    if !client.is_alive() {
        return Err("client exited".into());
    }
    Ok(Owner {
        sid: user(client.primary_token())?,
        auth: auth_id(client.primary_token())?,
        session: client.session,
    })
}
impl PrototypeEngine {
    fn approve_client(&self, client: &AuthenticatedClient) -> Result<()> {
        self.bundle.approve_manager(client.process.0)?;
        reject_loaded_runtime(client.process.0)?;
        for run in self.runs.values() {
            if run
                .session
                .job
                .as_ref()
                .ok_or("managed run missing Job")?
                .contains_process(client.process.0)
                .map_err(|e| e.to_string())?
            {
                return Err("managed target cannot delegate through management service".into());
            }
        }
        if !client.is_alive() {
            return Err("management peer exited or disconnected".into());
        }
        Ok(())
    }
    pub fn prepare() -> Result<Self> {
        crate::windows_service::validate_service_identity()?;
        let bundle = InstalledBundle::from_service_executable()?;
        let driver = DriverSession::open()?;
        Ok(Self {
            bundle,
            driver,
            runs: HashMap::new(),
            requests: HashMap::new(),
        })
    }
    pub fn launch(&mut self, client: &AuthenticatedClient, request: LaunchRequest) -> Result<Uuid> {
        self.approve_client(client)?;
        let identity = owner(client)?;
        // Direct Rust callers obey the same validation as wire requests.
        crate::protocol::decode(&serde_json::to_vec(&request).map_err(|e| e.to_string())?)?;
        let digest: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&serde_json::to_value(&request).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        )
        .into();
        if let Some(old) = self.requests.get(&request.request_id) {
            if old.digest != digest || old.owner != identity {
                return Err("request identity replay conflict".into());
            }
            return old.outcome.clone();
        }
        if self.runs.len() >= 64 || self.requests.len() >= 4096 {
            return Err("service registry capacity exhausted".into());
        }
        let request_id = request.request_id;
        self.requests.insert(
            request_id,
            RequestRecord {
                owner: identity.clone(),
                digest,
                outcome: Err("request is in progress".into()),
            },
        );
        let outcome = self.launch_new(client, request, identity);
        self.requests
            .get_mut(&request_id)
            .expect("reserved request")
            .outcome = outcome.clone();
        outcome
    }
    fn launch_new(
        &mut self,
        client: &AuthenticatedClient,
        mut request: LaunchRequest,
        identity: Owner,
    ) -> Result<Uuid> {
        if self
            .runs
            .values()
            .any(|r| r.request.container_id == request.container_id && r.owner != identity)
        {
            return Err("container belongs to a different owner".into());
        }
        if self
            .runs
            .values()
            .any(|r| r.request.instance_id == request.instance_id)
        {
            return Err("instance identity already used".into());
        }
        let snapshot = crate::snapshot::load_for_token(client.primary_token(), &request)?;
        let original_executable = request.executable.clone();
        let _input_leases = InstalledBundle::lease_input_path(&request.executable, false)?;
        let _cwd_leases = InstalledBundle::lease_input_path(&request.working_directory, true)?;
        let target = std::fs::canonicalize(&request.executable).map_err(|e| e.to_string())?;
        let _target_leases = self.bundle.assert_target_unmanaged(&target)?;
        request.executable = target;
        let id = request.instance_id;
        let req = SessionStartRequest {
            application_id: Uuid::new_v4(),
            launch: envbox_core::LaunchTarget::Executable {
                path: original_executable,
            },
            arguments: request.arguments.clone(),
            working_directory: Some(request.working_directory.clone()),
            profile: Some(snapshot.effective_profile.clone()),
            inherit_children: false,
            audit: true,
        };
        let driver = &self.driver;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut bind = |process, _pid, _creation_time| {
            if !client.is_alive() || std::time::Instant::now() >= deadline {
                return Err("client disconnected or launch deadline expired before binding".into());
            }
            driver.bind(process, &request, &snapshot)
        };
        let session = start_session_as_user(
            req,
            id,
            ServiceStartOptions {
                primary_token: client.primary_token(),
                runtime_bundle: &self.bundle.runtime_bundle,
                before_resume: &mut bind,
            },
        )
        .map_err(|e| e.to_string())?;
        self.runs.insert(
            id,
            ManagedRun {
                owner: identity,
                request,
                session,
                control_error: None,
            },
        );
        Ok(id)
    }
    pub fn status(
        &self,
        client: &AuthenticatedClient,
        id: Uuid,
    ) -> Result<(u32, envbox_core::InstanceStatus)> {
        self.approve_client(client)?;
        let identity = owner(client)?;
        let run = self.runs.get(&id).ok_or("unknown service run")?;
        if run.owner != identity {
            return Err("run belongs to another owner".into());
        }
        let stats = run
            .session
            .job
            .as_ref()
            .ok_or("service run has no Job ownership")?
            .stats()
            .map_err(|e| e.to_string())?;
        if stats.active_processes != 0 {
            if let Some(error) = &run.control_error {
                return Err(error.clone());
            }
        }
        let status = if stats.active_processes == 0 {
            envbox_core::InstanceStatus::Exited
        } else {
            run.session.instance.status
        };
        Ok((run.session.instance.root_pid, status))
    }
    pub fn stop(&mut self, client: &AuthenticatedClient, id: Uuid) -> Result<()> {
        self.approve_client(client)?;
        let identity = owner(client)?;
        let run = self.runs.get_mut(&id).ok_or("unknown service run")?;
        if run.owner != identity {
            return Err("run belongs to another owner".into());
        }
        if let Err(error) = run
            .session
            .job
            .as_ref()
            .ok_or("service run has no Job ownership")?
            .terminate()
        {
            let error = error.to_string();
            run.control_error = Some(error.clone());
            return Err(error);
        }
        run.control_error = None;
        run.session.instance.status = envbox_core::InstanceStatus::Stopping;
        Ok(())
    }
}

pub(crate) fn reject_loaded_runtime(process: windows::Win32::Foundation::HANDLE) -> Result<()> {
    use envbox_launcher::launcher::win::SafeHandle;
    use windows::Win32::{
        Foundation::{GetLastError, ERROR_NO_MORE_FILES},
        System::{Diagnostics::ToolHelp::*, Threading::GetProcessId},
    };
    unsafe {
        let pid = GetProcessId(process);
        if pid == 0 {
            return Err("invalid management process".into());
        }
        let snapshot = SafeHandle(
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)
                .map_err(|e| e.to_string())?,
        );
        let mut module = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        Module32FirstW(snapshot.0, &mut module).map_err(|e| e.to_string())?;
        loop {
            let len = module
                .szModule
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(module.szModule.len());
            let name = String::from_utf16_lossy(&module.szModule[..len]);
            if name.eq_ignore_ascii_case("envbox-runtime64.dll")
                || name.eq_ignore_ascii_case("envbox-runtime32.dll")
            {
                return Err("Runtime-loaded management process rejected".into());
            }
            if Module32NextW(snapshot.0, &mut module).is_err() {
                let code = GetLastError();
                if code != ERROR_NO_MORE_FILES {
                    return Err(format!("management module inspection failed: {}", code.0));
                }
                break;
            }
        }
    }
    Ok(())
}
