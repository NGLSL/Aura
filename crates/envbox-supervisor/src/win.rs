use super::*;
use std::collections::{HashMap, HashSet};
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
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

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

const REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const EXIT_WITH_GUI: &str = "--exit-with-gui";

fn gui_mutex_name() -> Vec<u16> {
    wide(&format!("{GUI_SINGLETON_NAME}.Mutex"))
}

fn mutex_exists(name: &[u16]) -> io::Result<bool> {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) } {
        Ok(handle) => {
            drop(Handle(handle));
            Ok(true)
        }
        Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(false),
        Err(error) => Err(error.into()),
    }
}

struct SupervisorLifetime {
    gui_mutex: Vec<u16>,
    saw_gui: bool,
    managers: HashMap<u32, Handle>,
}

impl SupervisorLifetime {
    fn idle_after_gui(&mut self) -> bool {
        self.managers.retain(|_, process| {
            let wait = unsafe { WaitForSingleObject(process.0, 0) };
            wait != WAIT_OBJECT_0
        });
        let Ok(gui_open) = mutex_exists(&self.gui_mutex) else {
            return false;
        };
        self.saw_gui |= gui_open;
        self.saw_gui && !gui_open && self.managers.is_empty()
    }
}

// The server owns an overlapped pipe; clients retain the existing NOWAIT
// handle. Every server operation has its own stable OVERLAPPED and event.
#[derive(Clone, Copy)]
enum PipeMode {
    Nonblocking,
    Overlapped,
}

struct PipeOperation {
    pipe: HANDLE,
    overlapped: Box<OVERLAPPED>,
    _event: Handle,
    pending: bool,
}

impl PipeOperation {
    fn new(pipe: HANDLE) -> io::Result<Self> {
        let event = Handle(unsafe { CreateEventW(None, true, false, None) }?);
        Ok(Self {
            pipe,
            overlapped: Box::new(OVERLAPPED {
                hEvent: event.0,
                ..Default::default()
            }),
            _event: event,
            pending: false,
        })
    }

    fn begin(&mut self, result: windows::core::Result<()>) -> io::Result<()> {
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.code() == ERROR_IO_PENDING.to_hresult() => {
                self.pending = true;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn wait(&mut self, duration: Duration) -> io::Result<Option<u32>> {
        // Round up so a sub-millisecond remainder cannot become a busy loop
        // immediately before the next refresh/deadline.
        let milliseconds = duration
            .as_nanos()
            .div_ceil(1_000_000)
            .min(u128::from(u32::MAX - 1)) as u32;
        match unsafe { WaitForSingleObject(self.overlapped.hEvent, milliseconds) } {
            WAIT_OBJECT_0 => {
                let mut transferred = 0;
                let result = unsafe {
                    GetOverlappedResult(self.pipe, &*self.overlapped, &mut transferred, false)
                };
                // A signaled event completes the operation, including errors.
                self.pending = false;
                result?;
                Ok(Some(transferred))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn finish(
        &mut self,
        result: windows::core::Result<()>,
        immediate_count: u32,
        end: Instant,
        cancel: Option<&AtomicBool>,
    ) -> io::Result<u32> {
        self.begin(result)?;
        check_cancel(cancel)?;
        if Instant::now() >= end {
            return Err(timeout());
        }
        if !self.pending {
            return Ok(immediate_count);
        }
        loop {
            check_cancel(cancel)?;
            let remaining = end
                .checked_duration_since(Instant::now())
                .ok_or_else(timeout)?;
            if let Some(count) = self.wait(remaining.min(REFRESH_INTERVAL))? {
                check_cancel(cancel)?;
                if Instant::now() >= end {
                    return Err(timeout());
                }
                return Ok(count);
            }
        }
    }
}

impl Drop for PipeOperation {
    fn drop(&mut self) {
        if self.pending {
            // CancelIoEx only requests cancellation. Drain it before freeing
            // OVERLAPPED, its event, or the caller-owned I/O buffer.
            unsafe {
                let _ = CancelIoEx(self.pipe, Some(&*self.overlapped));
                let mut transferred = 0;
                let _ = GetOverlappedResult(self.pipe, &*self.overlapped, &mut transferred, true);
            }
        }
    }
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
fn principal(process: HANDLE) -> io::Result<Principal> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token)?;
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

fn supervisor_mutex_name(owner: &Principal) -> Vec<u16> {
    wide(&format!(r"Local\AuraSupervisor-{}", scope(owner)))
}

fn image(process: HANDLE) -> io::Result<PathBuf> {
    unsafe {
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process,
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
        let process = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }?);
        Self::from_process(pid, process.0)
    }

    fn from_process(pid: u32, process: HANDLE) -> io::Result<Self> {
        unsafe {
            let mut creation = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user)?;
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

fn authenticate(pipe: HANDLE, owner: &Principal, config: &ServerConfig) -> io::Result<(u32, Handle)> {
    let mut pid = 0;
    unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) }?;
    let process = Handle(unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid)
    }?);
    if principal(process.0)? != *owner {
        return Err(denied("owner/integrity mismatch"));
    }
    if config
        .managed_targets
        .lock()
        .map_err(|_| denied("target registry unavailable"))?
        .contains(&ManagedTarget::from_process(pid, process.0)?)
        || has_runtime(pid)?
    {
        return Err(denied("Runtime targets cannot manage Supervisor"));
    }
    let path = image(process.0)?;
    let hash = file_hash(&path)?;
    if !config
        .approved_managers
        .iter()
        .any(|entry| entry.path == path && entry.sha256 == hash)
    {
        return Err(denied("unapproved management image"));
    }
    Ok((pid, process))
}

fn receive(
    pipe: HANDLE,
    end: Instant,
    cancel: Option<&AtomicBool>,
    mode: PipeMode,
) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    loop {
        check_cancel(cancel)?;
        if Instant::now() >= end {
            return Err(timeout());
        }
        let mut buffer = [0u8; 4096];
        let mut count = 0;
        let result = match mode {
            PipeMode::Nonblocking => {
                unsafe { ReadFile(pipe, Some(&mut buffer), Some(&mut count), None) }
                    .map_err(io::Error::from)
            }
            PipeMode::Overlapped => {
                let mut operation = PipeOperation::new(pipe)?;
                let result = unsafe {
                    ReadFile(
                        pipe,
                        Some(&mut buffer),
                        Some(&mut count),
                        Some(&mut *operation.overlapped),
                    )
                };
                operation
                    .finish(result, count, end, cancel)
                    .map(|read| count = read)
            }
        };
        if let Err(error) = result {
            let code = error.raw_os_error().map(|code| code as u32 & 0xffff);
            if code != Some(ERROR_NO_DATA.0) && code != Some(ERROR_PIPE_LISTENING.0) {
                return Err(error);
            }
            count = 0;
        }
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
        if matches!(mode, PipeMode::Nonblocking) || count == 0 {
            pause(end, cancel)?;
        }
    }
}
fn send<T: Serialize>(
    pipe: HANDLE,
    value: &T,
    end: Instant,
    cancel: Option<&AtomicBool>,
    mode: PipeMode,
) -> io::Result<()> {
    check_cancel(cancel)?;
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    let mut offset = 0;
    while offset < bytes.len() {
        check_cancel(cancel)?;
        if Instant::now() >= end {
            return Err(timeout());
        }
        let mut count = 0;
        match mode {
            PipeMode::Nonblocking => {
                unsafe { WriteFile(pipe, Some(&bytes[offset..]), Some(&mut count), None) }?
            }
            PipeMode::Overlapped => {
                let mut operation = PipeOperation::new(pipe)?;
                let result = unsafe {
                    WriteFile(
                        pipe,
                        Some(&bytes[offset..]),
                        Some(&mut count),
                        Some(&mut *operation.overlapped),
                    )
                };
                count = operation.finish(result, count, end, cancel)?;
            }
        }
        offset += count as usize;
        if offset < bytes.len() {
            pause(end, cancel)?;
        }
    }
    Ok(())
}

pub fn run_default() -> io::Result<()> {
    let exit_with_gui = std::env::args_os().skip(1).any(|arg| arg == EXIT_WITH_GUI);
    serve_inner(ServerConfig::production()?, exit_with_gui)
}

/// Serves until explicitly stopped or the GUI has closed and all tracked work has ended.
pub fn serve(config: ServerConfig) -> io::Result<()> {
    serve_inner(config, false)
}

fn serve_inner(config: ServerConfig, exit_with_gui: bool) -> io::Result<()> {
    if has_runtime(std::process::id())? {
        return Err(denied("Runtime cannot host Supervisor management"));
    }
    let owner = principal(unsafe { GetCurrentProcess() })?;
    let mutex_name = supervisor_mutex_name(&owner);
    let _lock = Handle(unsafe { CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr())) }?);
    // Last-error is read immediately; an existing owner (including an abandoned
    // mutex that remains open) is not taken over by a second server.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Ok(());
    }
    let name = wide(&pipe_name(&owner));
    let pipe = Handle(unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
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
    let gui_mutex = gui_mutex_name();
    let mut lifetime = SupervisorLifetime {
        saw_gui: exit_with_gui || mutex_exists(&gui_mutex).unwrap_or(false),
        gui_mutex,
        managers: HashMap::new(),
    };
    let generation = uuid::Uuid::new_v4().to_string();
    let mut runs = crate::runs::Runs::new(
        config.store.clone(),
        config.managed_targets.clone(),
        generation.clone(),
    );
    let mut refreshed = Instant::now();
    'accept: while !config.stop.load(Ordering::Acquire) {
        if refreshed.elapsed() >= REFRESH_INTERVAL {
            runs.refresh();
            lifetime.idle_after_gui();
            refreshed = Instant::now();
        }
        let mut operation = PipeOperation::new(pipe.0)?;
        let connected = unsafe { ConnectNamedPipe(pipe.0, Some(&mut *operation.overlapped)) };
        if let Err(error) = connected {
            match WIN32_ERROR(error.code().0 as u32 & 0xffff) {
                ERROR_PIPE_CONNECTED => {}
                ERROR_IO_PENDING => {
                    operation.pending = true;
                    loop {
                        if config.stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let wait = REFRESH_INTERVAL.saturating_sub(refreshed.elapsed());
                        match operation.wait(wait) {
                            Ok(Some(_)) => break,
                            Ok(None) => {
                                runs.refresh();
                                refreshed = Instant::now();
                                // Prefer a waiting connection over idle
                                // shutdown and finish accepted work first.
                                if lifetime.idle_after_gui() && !runs.has_tracked_jobs() {
                                    return Ok(());
                                }
                            }
                            Err(error)
                                if error.raw_os_error().is_some_and(|code| {
                                    matches!(
                                        WIN32_ERROR(code as u32 & 0xffff),
                                        ERROR_NO_DATA | ERROR_BROKEN_PIPE
                                    )
                                }) =>
                            {
                                unsafe {
                                    let _ = DisconnectNamedPipe(pipe.0);
                                }
                                continue 'accept;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                ERROR_NO_DATA => {
                    // A client can close before accept observes it. Reset the
                    // pipe and continue serving other managers.
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
        let request = receive(pipe.0, end, Some(&config.stop), PipeMode::Overlapped)
            .and_then(|bytes| serde_json::from_slice::<Request>(&bytes).map_err(Into::into));
        if let Ok(request) = request {
            let is_authenticated = authenticated.is_ok();
            if request.version == PROTOCOL_VERSION {
                if let Ok((pid, process)) = authenticated {
                    // Keep an approved manager alive between Ping and its next
                    // request, even if Aura closes between the two connections.
                    lifetime.managers.insert(pid, process);
                }
            }
            let status = if !is_authenticated {
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
                "Run" | "Detect" | "RunStatus" | "List" | "Stop" | "StopAll"
            ) && request.generation.is_none()
            {
                "GenerationRequired"
            } else if request.command == "Ping" {
                "Ok"
            } else {
                "Unsupported"
            };
            let (status, run, instances) = if is_authenticated
                && request.version == PROTOCOL_VERSION
                && request
                    .generation
                    .as_ref()
                    .is_some_and(|expected| expected == &generation)
                && matches!(request.command.as_str(), "Run" | "Detect" | "RunStatus")
            {
                match request.run.as_ref() {
                    Some(_) if uuid::Uuid::parse_str(&request.request_id).is_err() => {
                        ("InvalidRequest".into(), None, vec![])
                    }
                    Some(command) if request.command == "Detect" => match request.detection {
                        Some(site) => {
                            let (status, result) = runs.detect(command, site, &request.request_id);
                            (status, result, vec![])
                        }
                        None => ("InvalidRequest".into(), None, vec![]),
                    },
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
            } else if is_authenticated
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
            let _ = send(
                pipe.0,
                &response,
                end,
                Some(&config.stop),
                PipeMode::Overlapped,
            );
            // Give the bounded client time to consume the response. Disconnect
            // after an acknowledged close, with a strict server deadline.
            let _ = receive(pipe.0, end, Some(&config.stop), PipeMode::Overlapped);
        }
        unsafe {
            let _ = DisconnectNamedPipe(pipe.0);
        }
    }
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
        Ok(pipe_name(&principal(unsafe { GetCurrentProcess() })?))
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
        let owner = principal(unsafe { GetCurrentProcess() })?;
        let mutex_name = supervisor_mutex_name(&owner);
        let mut child: Option<std::process::Child> = None;
        loop {
            let ping = connect_until(&owner, &self.executable, end, cancel)
                .and_then(|pipe| exchange(pipe, self.ping_request(None), end, cancel));
            match ping {
                Ok(response) => return Ok(response),
                Err(error) if retryable(&error) => {}
                Err(error) => return Err(error),
            }
            if let Some(process) = &mut child {
                if let Some(status) = process.try_wait()? {
                    if !status.success() {
                        return Err(io::Error::other(format!("Supervisor exited: {status}")));
                    }
                    child = None;
                }
            }
            // While a previous owner is closing its pipe (or a concurrent
            // starter owns the mutex), wait for that transition to finish.
            if child.is_none() && !mutex_exists(&mutex_name)? {
                let exit_with_gui = mutex_exists(&gui_mutex_name()).unwrap_or(false);
                child = Some(
                    std::process::Command::new(&self.executable)
                        .args(exit_with_gui.then_some(EXIT_WITH_GUI))
                        .creation_flags(CREATE_NO_WINDOW.0)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()?,
                );
            }
            pause(end, cancel)?;
        }
    }
    fn ping_request(&self, generation: Option<String>) -> Request {
        Request {
            detection: None,
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
                &principal(unsafe { GetCurrentProcess() })?,
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
    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::BrokenPipe)
        || error.raw_os_error().is_some_and(|code| {
            matches!(
                WIN32_ERROR(code as u32 & 0xffff),
                ERROR_FILE_NOT_FOUND
                    | ERROR_PIPE_BUSY
                    | ERROR_BROKEN_PIPE
                    | ERROR_NO_DATA
                    | ERROR_PIPE_NOT_CONNECTED
            )
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
    let server = match unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid)
    } {
        Ok(handle) => Handle(handle),
        Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
            return Err(io::Error::from_raw_os_error(ERROR_BROKEN_PIPE.0 as i32));
        }
        Err(error) => return Err(error.into()),
    };
    let authentication = (|| {
        if principal(server.0)? != *owner {
            return Err(denied("Supervisor owner mismatch"));
        }
        if has_runtime(pid)? {
            return Err(denied(
                "injected process cannot serve Supervisor management",
            ));
        }
        let path = image(server.0)?;
        if path != std::fs::canonicalize(expected_server)?
            || file_hash(&path)? != file_hash(expected_server)?
        {
            return Err(denied("Supervisor artifact mismatch"));
        }
        Ok(())
    })();
    if let Err(error) = authentication {
        if unsafe { WaitForSingleObject(server.0, 0) } == WAIT_OBJECT_0 {
            return Err(io::Error::from_raw_os_error(ERROR_BROKEN_PIPE.0 as i32));
        }
        return Err(error);
    }
    Ok(pipe)
}
fn exchange(
    pipe: Handle,
    request: Request,
    end: Instant,
    cancel: Option<&AtomicBool>,
) -> io::Result<Response> {
    send(pipe.0, &request, end, cancel, PipeMode::Nonblocking)?;
    let bytes = receive(pipe.0, end, cancel, PipeMode::Nonblocking)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_gui_waits_for_managers_and_reopening_keeps_supervisor() {
        let name = wide(&format!(r"Local\AuraGuiLifecycleTest-{}", uuid::Uuid::new_v4()));
        let open_gui = || {
            Handle(unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.unwrap())
        };
        let mut lifetime = SupervisorLifetime {
            gui_mutex: name.clone(),
            saw_gui: false,
            managers: HashMap::new(),
        };
        assert!(!lifetime.idle_after_gui(), "standalone CLI lifetime");
        let gui = open_gui();
        assert!(!lifetime.idle_after_gui());

        // A process handle becomes signaled on exit. A manually signaled
        // event exercises the same wait semantics without starting a manager.
        let manager = Handle(unsafe { CreateEventW(None, true, false, None) }.unwrap());
        let manager_exit = manager.0;
        lifetime.managers.insert(1, manager);
        drop(gui);
        assert!(!lifetime.idle_after_gui(), "manager is between requests");
        unsafe { SetEvent(manager_exit) }.unwrap();
        assert!(lifetime.idle_after_gui());
        assert!(lifetime.managers.is_empty());

        let gui = open_gui();
        assert!(!lifetime.idle_after_gui());
        drop(gui);
        assert!(lifetime.idle_after_gui());
    }
}
