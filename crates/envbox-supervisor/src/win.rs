use super::*;
use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Instant;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::*;

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn denied(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, s)
}
fn timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Supervisor deadline expired")
}
fn check_cancel(cancel: Option<&AtomicBool>) -> io::Result<()> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Supervisor operation cancelled",
        ));
    }
    Ok(())
}
fn pause(end: Instant, cancel: Option<&AtomicBool>) -> io::Result<()> {
    check_cancel(cancel)?;
    if Instant::now() >= end {
        return Err(timeout());
    }
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Principal {
    sid: Vec<u8>,
    integrity: u32,
}
unsafe fn token_data(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<usize>> {
    let mut size = 0;
    let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut size) };
    if size == 0 || size > 65536 {
        return Err(denied("invalid token"));
    }
    let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        GetTokenInformation(
            token,
            class,
            Some(data.as_mut_ptr().cast()),
            size,
            &mut size,
        )
    }?;
    Ok(data)
}
fn principal(pid: u32) -> io::Result<Principal> {
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token)?;
        let token = Handle(token);
        let user = token_data(token.0, TokenUser)?;
        let sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let length = GetLengthSid(sid) as usize;
        if length == 0 || length > 1024 {
            return Err(denied("invalid SID"));
        }
        let bytes = std::slice::from_raw_parts(sid.0.cast::<u8>(), length).to_vec();
        let label = token_data(token.0, TokenIntegrityLevel)?;
        let sid = (*(label.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()))
            .Label
            .Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(denied("invalid integrity"));
        }
        Ok(Principal {
            sid: bytes,
            integrity: *GetSidSubAuthority(sid, u32::from(count - 1)),
        })
    }
}
fn scope(owner: &Principal) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}-{:x}", Sha256::digest(&owner.sid), owner.integrity)
}
fn pipe_name(owner: &Principal) -> String {
    format!(r"\\.\pipe\aura-supervisor-{}", scope(owner))
}

fn image(pid: u32) -> io::Result<PathBuf> {
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )?;
        std::fs::canonicalize(PathBuf::from(String::from_utf16_lossy(
            &buffer[..size as usize],
        )))
    }
}
fn has_runtime(pid: u32) -> io::Result<bool> {
    unsafe {
        // Inability to inspect modules denies management, including cross-bitness
        // inspection that Windows cannot establish safely.
        let snapshot = Handle(CreateToolhelp32Snapshot(
            TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32,
            pid,
        )?);
        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        Module32FirstW(snapshot.0, &mut entry)?;
        loop {
            let length = entry
                .szModule
                .iter()
                .position(|x| *x == 0)
                .unwrap_or(entry.szModule.len());
            let name = String::from_utf16_lossy(&entry.szModule[..length]).to_ascii_lowercase();
            if name == "envbox-runtime64.dll" || name == "envbox-runtime32.dll" {
                return Ok(true);
            }
            if Module32NextW(snapshot.0, &mut entry).is_err() {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(denied("module inspection failed"));
                }
                break;
            }
        }
        Ok(false)
    }
}

pub struct ServerConfig {
    pub store: envbox_storage::ConfigStore,
    pub approved_managers: Vec<ApprovedManager>,
    /// Server-side deny registry populated by the future instance owner.
    pub managed_targets: Arc<Mutex<HashSet<ManagedTarget>>>,
    pub stop: Arc<AtomicBool>,
    pub request_timeout: Duration,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ManagedTarget {
    pub pid: u32,
    pub creation_time: u64,
}
impl ManagedTarget {
    pub fn observe(pid: u32) -> io::Result<Self> {
        unsafe {
            let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
            let mut creation = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user)?;
            Ok(Self {
                pid,
                creation_time: (u64::from(creation.dwHighDateTime) << 32)
                    | u64::from(creation.dwLowDateTime),
            })
        }
    }
}
impl ServerConfig {
    pub fn production() -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        let directory = executable
            .parent()
            .ok_or_else(|| denied("missing artifact directory"))?;
        let approved_managers = ["envbox.exe", "Aura.exe", "envbox-app.exe"]
            .iter()
            .filter_map(|name| ApprovedManager::from_file(&directory.join(name)).ok())
            .collect();
        Ok(Self {
            store: envbox_storage::ConfigStore::new(envbox_storage::ConfigStore::default_root()),
            approved_managers,
            managed_targets: Arc::default(),
            stop: Arc::default(),
            request_timeout: DEFAULT_TIMEOUT,
        })
    }
}

fn authenticate(pipe: HANDLE, owner: &Principal, config: &ServerConfig) -> io::Result<()> {
    let mut pid = 0;
    unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) }?;
    if principal(pid)? != *owner {
        return Err(denied("owner/integrity mismatch"));
    }
    if config
        .managed_targets
        .lock()
        .map_err(|_| denied("target registry unavailable"))?
        .contains(&ManagedTarget::observe(pid)?)
        || has_runtime(pid)?
    {
        return Err(denied("Runtime targets cannot manage Supervisor"));
    }
    let path = image(pid)?;
    let hash = file_hash(&path)?;
    if !config
        .approved_managers
        .iter()
        .any(|entry| entry.path == path && entry.sha256 == hash)
    {
        return Err(denied("unapproved management image"));
    }
    Ok(())
}

fn receive(pipe: HANDLE, end: Instant, cancel: Option<&AtomicBool>) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    loop {
        check_cancel(cancel)?;
        let mut buffer = [0u8; 4096];
        let mut count = 0;
        let result = unsafe { ReadFile(pipe, Some(&mut buffer), Some(&mut count), None) };
        if count > 0 {
            data.extend_from_slice(&buffer[..count as usize]);
        }
        if data.len() > 65536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request too large",
            ));
        }
        if let Some(index) = data.iter().position(|byte| *byte == b'\n') {
            data.truncate(index);
            return Ok(data);
        }
        if let Err(error) = result {
            let code = error.code().0 as u32 & 0xffff;
            if code != ERROR_NO_DATA.0 && code != ERROR_PIPE_LISTENING.0 {
                return Err(error.into());
            }
        }
        pause(end, cancel)?;
    }
}
fn send<T: Serialize>(
    pipe: HANDLE,
    value: &T,
    end: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    check_cancel(cancel)?;
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    let mut offset = 0;
    while offset < bytes.len() {
        let mut count = 0;
        unsafe { WriteFile(pipe, Some(&bytes[offset..]), Some(&mut count), None) }?;
        offset += count as usize;
        if offset < bytes.len() {
            pause(end, cancel)?;
        }
    }
    Ok(())
}

pub fn run_default() -> io::Result<()> {
    serve(ServerConfig::production()?)
}

/// Runs until `stop` is set. Client disconnects never stop the owner process.
pub fn serve(config: ServerConfig) -> io::Result<()> {
    if has_runtime(std::process::id())? {
        return Err(denied("Runtime cannot host Supervisor management"));
    }
    let owner = principal(std::process::id())?;
    let mutex_name = wide(&format!(r"Local\AuraSupervisor-{}", scope(&owner)));
    let lock = Handle(unsafe { CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr())) }?);
    // Last-error is read immediately; an existing owner (including an abandoned
    // mutex that remains open) is not taken over by a second server.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Ok(());
    }
    let name = wide(&pipe_name(&owner));
    let pipe = Handle(unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            None,
        )
    });
    if pipe.0 == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let generation = uuid::Uuid::new_v4().to_string();
    let mut runs = crate::runs::Runs::new(
        config.store.clone(),
        config.managed_targets.clone(),
        generation.clone(),
    );
    let mut refreshed = Instant::now();
    while !config.stop.load(Ordering::Acquire) {
        if refreshed.elapsed() >= Duration::from_millis(250) {
            runs.refresh();
            refreshed = Instant::now();
        }
        let connected = unsafe { ConnectNamedPipe(pipe.0, None) };
        if let Err(error) = connected {
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => {}
                ERROR_PIPE_LISTENING => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                ERROR_NO_DATA => {
                    // A client can close before the nonblocking accept observes
                    // it. Reset this pipe rather than spin forever on NO_DATA.
                    unsafe {
                        let _ = DisconnectNamedPipe(pipe.0);
                    }
                    continue;
                }
                _ => return Err(error.into()),
            }
        }
        let end = Instant::now() + config.request_timeout;
        runs.refresh();
        let authenticated = authenticate(pipe.0, &owner, &config);
        #[cfg(debug_assertions)]
        if let Err(error) = &authenticated {
            eprintln!("Supervisor authentication: {error}");
        }
        let request = receive(pipe.0, end, Some(&config.stop))
            .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).map_err(Into::into));
        if let Ok(request) = request {
            let status = if authenticated.is_err() {
                "AuthenticationDenied"
            } else if request.version != PROTOCOL_VERSION {
                "VersionMismatch"
            } else if request
                .generation
                .as_ref()
                .is_some_and(|expected| expected != &generation)
            {
                "GenerationMismatch"
            } else if matches!(
                request.command.as_str(),
                "Run" | "RunStatus" | "List" | "Stop" | "StopAll"
            ) && request.generation.is_none()
            {
                "GenerationRequired"
            } else if request.command == "Ping" {
                "Ok"
            } else {
                "Unsupported"
            };
            let (status, run, instances) = if authenticated.is_ok()
                && request.version == PROTOCOL_VERSION
                && request
                    .generation
                    .as_ref()
                    .is_some_and(|expected| expected == &generation)
                && matches!(request.command.as_str(), "Run" | "RunStatus")
            {
                match request.run.as_ref() {
                    Some(_) if uuid::Uuid::parse_str(&request.request_id).is_err() => {
                        ("InvalidRequest".into(), None, vec![])
                    }
                    Some(command) => {
                        let (status, result) = if request.command == "Run" {
                            runs.run(&request.request_id, command)
                        } else {
                            runs.query(command)
                        };
                        (status, Some(result), vec![])
                    }
                    None => ("InvalidRequest".into(), None, vec![]),
                }
            } else if authenticated.is_ok()
                && request.version == PROTOCOL_VERSION
                && request
                    .generation
                    .as_ref()
                    .is_some_and(|expected| expected == &generation)
                && matches!(request.command.as_str(), "List" | "Stop" | "StopAll")
            {
                if uuid::Uuid::parse_str(&request.request_id).is_err() {
                    ("InvalidRequest".into(), None, vec![])
                } else {
                    let (status, instances) = runs.control(
                        &request.request_id,
                        &request.command,
                        request.container_id,
                        request.run.as_ref(),
                    );
                    (status, None, instances)
                }
            } else {
                (status.to_owned(), None, vec![])
            };
            let response = Response {
                version: PROTOCOL_VERSION,
                generation: generation.clone(),
                request_id: request.request_id,
                supervisor_pid: std::process::id(),
                status,
                run,
                instances,
            };
            let end = Instant::now() + config.request_timeout;
            let _ = send(pipe.0, &response, end, Some(&config.stop));
            // Give the bounded client time to consume the response. Disconnect
            // after an acknowledged close, with a strict server deadline.
            let _ = receive(pipe.0, end, Some(&config.stop));
        }
        unsafe {
            let _ = DisconnectNamedPipe(pipe.0);
        }
    }
    drop(lock);
    Ok(())
}

#[derive(Clone)]
pub struct SupervisorClient {
    pub executable: PathBuf,
    pub timeout: Duration,
}
impl SupervisorClient {
    /// Diagnostic endpoint, derived exclusively from the caller's OS token.
    pub fn endpoint(&self) -> io::Result<String> {
        Ok(pipe_name(&principal(std::process::id())?))
    }
    pub fn beside_current_executable() -> io::Result<Self> {
        let executable = std::env::current_exe()?.with_file_name("envbox-supervisor.exe");
        Ok(Self {
            executable,
            timeout: DEFAULT_TIMEOUT,
        })
    }
    pub fn ensure_started(&self) -> io::Result<Response> {
        self.ensure_started_cancellable(None)
    }
    pub fn ensure_started_cancellable(&self, cancel: Option<&AtomicBool>) -> io::Result<Response> {
        check_cancel(cancel)?;
        if has_runtime(std::process::id())? {
            return Err(denied("Runtime targets cannot start management Supervisor"));
        }
        let end = Instant::now() + self.timeout;
        let owner = principal(std::process::id())?;
        // Avoid spawning for authentication/version failures.
        match connect_until(&owner, &self.executable, end, cancel) {
            Ok(pipe) => return exchange(pipe, self.ping_request(None), end, cancel),
            Err(error) if retryable(&error) => {}
            Err(error) => return Err(error),
        }
        let mut child = std::process::Command::new(&self.executable)
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        loop {
            match connect_until(&owner, &self.executable, end, cancel) {
                Ok(pipe) => return exchange(pipe, self.ping_request(None), end, cancel),
                Err(error) if retryable(&error) => {}
                Err(error) => return Err(error),
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(io::Error::other(format!("Supervisor exited: {status}")));
                }
                // A concurrent startup loser exits successfully; the winning
                // server may still be creating its pipe.
            }
            pause(end, cancel)?;
        }
    }
    fn ping_request(&self, generation: Option<String>) -> Request {
        Request {
            version: PROTOCOL_VERSION,
            generation,
            request_id: uuid::Uuid::new_v4().to_string(),
            command: "Ping".into(),
            run: None,
            container_id: None,
        }
    }
    pub fn request(&self, request: Request) -> io::Result<Response> {
        self.request_cancellable(request, None)
    }
    pub fn request_cancellable(
        &self,
        request: Request,
        cancel: Option<&AtomicBool>,
    ) -> io::Result<Response> {
        check_cancel(cancel)?;
        let end = Instant::now() + self.timeout;
        exchange(
            connect_until(
                &principal(std::process::id())?,
                &self.executable,
                end,
                cancel,
            )?,
            request,
            end,
            cancel,
        )
    }
}
fn retryable(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.raw_os_error().is_some_and(|code| {
            (code as u32 & 0xffff) == ERROR_PIPE_BUSY.0
                || (code as u32 & 0xffff) == ERROR_FILE_NOT_FOUND.0
        })
}
fn connect_until(
    owner: &Principal,
    expected_server: &Path,
    end: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<Handle> {
    loop {
        check_cancel(cancel)?;
        match connect(owner, expected_server) {
            Err(error)
                if error
                    .raw_os_error()
                    .is_some_and(|code| (code as u32 & 0xffff) == ERROR_PIPE_BUSY.0) =>
            {
                pause(end, cancel)?
            }
            result => return result,
        }
    }
}
fn connect(owner: &Principal, expected_server: &Path) -> io::Result<Handle> {
    let name = wide(&pipe_name(owner));
    let pipe = Handle(unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }?);
    let mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
    unsafe { SetNamedPipeHandleState(pipe.0, Some(&mode), None, None) }?;
    // A rogue server cannot return a believable authenticated response.
    let mut pid = 0;
    unsafe { GetNamedPipeServerProcessId(pipe.0, &mut pid) }?;
    if principal(pid)? != *owner {
        return Err(denied("Supervisor owner mismatch"));
    }
    if has_runtime(pid)? {
        return Err(denied(
            "injected process cannot serve Supervisor management",
        ));
    }
    let path = image(pid)?;
    if path != std::fs::canonicalize(expected_server)?
        || file_hash(&path)? != file_hash(expected_server)?
    {
        return Err(denied("Supervisor artifact mismatch"));
    }
    Ok(pipe)
}
fn exchange(
    pipe: Handle,
    request: Request,
    end: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<Response> {
    send(pipe.0, &request, end, cancel)?;
    let bytes = receive(pipe.0, end, cancel)?;
    let response: Response = serde_json::from_slice(&bytes)?;
    if response.request_id != request.request_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request identity mismatch",
        ));
    }
    let mut server_pid = 0;
    unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server_pid) }?;
    if response.supervisor_pid != server_pid {
        return Err(denied("Supervisor PID mismatch"));
    }
    if response.status == "AuthenticationDenied" {
        return Err(denied("Supervisor authentication denied"));
    }
    if response.version != PROTOCOL_VERSION
        || response.status == "VersionMismatch"
        || response.status == "GenerationMismatch"
        || response.status == "GenerationRequired"
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, response.status));
    }
    if request
        .generation
        .is_some_and(|expected| expected != response.generation)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stale generation",
        ));
    }
    Ok(response)
}
