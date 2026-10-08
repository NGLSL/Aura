//! Host-side Named Pipe server for Runtime IPC Bootstrap (ticket 40).
//!
//! Serves `\\.\pipe\envbox-runtime` (override `ENVBOX_IPC_PIPE`). Line protocol
//! is defined in `runtime/src/ipc_bootstrap.h` / [`crate::ipc`].

use crate::ipc::{IpcMessage, SessionTable};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Shared host session registry used by the pipe server.
pub type SharedTable = Arc<Mutex<SessionTable>>;

#[derive(Debug, Clone)]
pub(crate) struct AuthenticatedProcess {
    pub pid: u32,
    pub creation_time: u64,
}

pub(crate) fn protocol_denied(reason: &str) -> IpcMessage {
    IpcMessage::Other {
        name: "ERROR".into(),
        fields: vec![("code".into(), reason.into())],
    }
}

#[cfg(windows)]
pub(crate) fn process_creation_time(pid: u32) -> Option<u64> {
    use crate::launcher::win::SafeHandle;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let process = SafeHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user).ok()?;
        Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

#[cfg(not(windows))]
pub(crate) fn process_creation_time(_pid: u32) -> Option<u64> {
    None
}

#[cfg(windows)]
fn authenticate_process(
    pid: u32,
    expected: Option<&crate::service_start::TokenOwner>,
) -> std::io::Result<AuthenticatedProcess> {
    use crate::launcher::win::SafeHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetLengthSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
        TokenIntegrityLevel, TokenUser, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe fn token_info(
        token: HANDLE,
        class: TOKEN_INFORMATION_CLASS,
    ) -> std::io::Result<Vec<usize>> {
        let mut size = 0;
        let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut size) };
        if size == 0 || size > 65536 {
            return Err(std::io::Error::last_os_error());
        }
        // Windows returns structures containing pointers: maintain alignment.
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
    unsafe fn facts(pid: u32) -> std::io::Result<(Vec<u8>, u32)> {
        let process =
            SafeHandle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }?);
        let mut raw = HANDLE::default();
        unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut raw) }?;
        let token = SafeHandle(raw);
        let user = unsafe { token_info(token.0, TokenUser) }?;
        let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let len = unsafe { GetLengthSid(sid) } as usize;
        if len == 0 || len > 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "invalid owner SID",
            ));
        }
        let sid_bytes = unsafe { std::slice::from_raw_parts(sid.0.cast::<u8>(), len) }.to_vec();
        let integrity = unsafe { token_info(token.0, TokenIntegrityLevel) }?;
        let sid = unsafe {
            (*(integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()))
                .Label
                .Sid
        };
        let count = unsafe { *GetSidSubAuthorityCount(sid) };
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "invalid integrity SID",
            ));
        }
        let level = unsafe { *GetSidSubAuthority(sid, u32::from(count - 1)) };
        Ok((sid_bytes, level))
    }
    if let Some(expected) = expected {
        let process = crate::launcher::win::SafeHandle(unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
        }?);
        let mut raw = HANDLE::default();
        unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut raw) }?;
        let token = crate::launcher::win::SafeHandle(raw);
        let peer = crate::service_start::TokenOwner::read(token.0, false)?;
        if !expected.admits(&peer) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "bootstrap user/integrity/session mismatch",
            ));
        }
        return Ok(AuthenticatedProcess {
            pid,
            creation_time: process_creation_time(pid)
                .ok_or_else(|| std::io::Error::other("client generation unavailable"))?,
        });
    }
    let client = unsafe { facts(pid) }?;
    let owner = unsafe { facts(std::process::id()) }?;
    if client.0 != owner.0 || client.1 > owner.1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "bootstrap owner/integrity mismatch",
        ));
    }
    let creation_time = process_creation_time(pid).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "cannot query client generation",
        )
    })?;
    Ok(AuthenticatedProcess { pid, creation_time })
}

#[cfg(windows)]
pub(crate) fn process_parent(pid: u32) -> Option<u32> {
    use crate::launcher::win::SafeHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snapshot = SafeHandle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        Process32FirstW(snapshot.0, &mut entry).ok()?;
        loop {
            if entry.th32ProcessID == pid {
                return Some(entry.th32ParentProcessID);
            }
            if Process32NextW(snapshot.0, &mut entry).is_err() {
                return None;
            }
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn process_parent(_pid: u32) -> Option<u32> {
    None
}

pub(crate) fn file_sha256(path: &std::path::Path) -> Result<String, crate::ipc::IpcError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| crate::ipc::IpcError::Io(e.to_string()))?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file
            .read(&mut bytes)
            .map_err(|e| crate::ipc::IpcError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(windows)]
pub(crate) fn process_has_module(pid: u32, expected: &std::path::Path) -> bool {
    use crate::launcher::win::SafeHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE,
        TH32CS_SNAPMODULE32,
    };
    unsafe {
        let Ok(handle) = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)
        else {
            return false;
        };
        let snapshot = SafeHandle(handle);
        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        if Module32FirstW(snapshot.0, &mut entry).is_err() {
            return false;
        }
        loop {
            let end = entry
                .szExePath
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExePath.len());
            let path = std::path::PathBuf::from(String::from_utf16_lossy(&entry.szExePath[..end]));
            if std::fs::canonicalize(path).ok().as_deref() == Some(expected) {
                return true;
            }
            if Module32NextW(snapshot.0, &mut entry).is_err() {
                return false;
            }
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn process_has_module(_pid: u32, _expected: &std::path::Path) -> bool {
    false
}

/// Background Named Pipe broker. Dropping stops the accept loop.
pub struct HostBroker {
    table: SharedTable,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    pipe_name: String,
    #[cfg(windows)]
    _owner: PipeOwner,
    #[cfg(test)]
    pause: Option<Arc<AcceptPause>>,
}

#[cfg(windows)]
struct PipeOwner(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for PipeOwner {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn claim_pipe_name(name: &str) -> std::io::Result<PipeOwner> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    // A mutex reserves the name even while the server replaces a completed
    // pipe instance. The hash keeps the Windows object name path-independent.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in name.bytes().map(|b| b.to_ascii_lowercase()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
    let object_name = format!(r"Local\AuraPipe-{hash:016x}");
    let wide: Vec<u16> = std::ffi::OsStr::new(&object_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(wide.as_ptr()))?;
        let already_exists = GetLastError() == ERROR_ALREADY_EXISTS;
        if already_exists {
            let _ = CloseHandle(handle);
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("pipe {name} is already owned by another Aura session"),
            ));
        }
        Ok(PipeOwner(handle))
    }
}

impl HostBroker {
    /// Start serving on the default (or `ENVBOX_IPC_PIPE`) pipe name.
    pub fn start(table: SharedTable) -> std::io::Result<Self> {
        Self::start_on(table, pipe_name())
    }

    /// Start on an explicit pipe path (per-session names avoid cross-test races).
    pub fn start_on(table: SharedTable, pipe_name: String) -> std::io::Result<Self> {
        Self::start_on_impl(
            table,
            pipe_name,
            None,
            #[cfg(test)]
            None,
        )
    }

    pub(crate) fn start_on_owned(
        table: SharedTable,
        pipe_name: String,
        #[cfg(windows)] expected: Option<crate::service_start::TokenOwner>,
        #[cfg(not(windows))] expected: Option<()>,
    ) -> std::io::Result<Self> {
        Self::start_on_impl(
            table,
            pipe_name,
            expected,
            #[cfg(test)]
            None,
        )
    }

    fn start_on_impl(
        table: SharedTable,
        pipe_name: String,
        #[cfg(windows)] expected: Option<crate::service_start::TokenOwner>,
        #[cfg(not(windows))] expected: Option<()>,
        #[cfg(test)] pause: Option<Arc<AcceptPause>>,
    ) -> std::io::Result<Self> {
        #[cfg(windows)]
        let owner = claim_pipe_name(&pipe_name)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let table2 = table.clone();
        let name2 = pipe_name.clone();
        // Do not return until the accept loop has created its first pipe
        // instance. Runtime DLL initialization happens immediately after the
        // launcher returns from this function; without this handoff the
        // client can spend its first retry interval waiting for a server
        // thread that has not been scheduled yet.
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        #[cfg(test)]
        let pause2 = pause.clone();
        let join = std::thread::Builder::new()
            .name("envbox-ipc-host".into())
            .spawn(move || {
                serve_loop(
                    table2,
                    stop2,
                    name2,
                    expected,
                    Some(ready_tx),
                    #[cfg(test)]
                    pause2,
                )
            })?;

        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = join.join();
                return Err(err);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                stop.store(true, Ordering::SeqCst);
                nudge_pipe(&pipe_name);
                let _ = join.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "IPC broker did not become ready",
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                stop.store(true, Ordering::SeqCst);
                nudge_pipe(&pipe_name);
                let _ = join.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "IPC broker stopped before becoming ready",
                ));
            }
        }
        Ok(Self {
            table,
            stop,
            join: Some(join),
            pipe_name,
            #[cfg(windows)]
            _owner: owner,
            #[cfg(test)]
            pause,
        })
    }

    /// Pipe path clients should use (`ENVBOX_IPC_PIPE`).
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    pub fn table(&self) -> SharedTable {
        self.table.clone()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Nudge a blocked ConnectNamedPipe by opening the pipe as a client.
        nudge_pipe(&self.pipe_name);
        #[cfg(test)]
        if let Some(pause) = &self.pause {
            let _ = pause.nudged.send(());
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[cfg(test)]
struct AcceptPause {
    entered: std::sync::mpsc::SyncSender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    published: std::sync::mpsc::SyncSender<()>,
    nudged: std::sync::mpsc::Sender<()>,
}

impl Drop for HostBroker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn pipe_name() -> String {
    match std::env::var("ENVBOX_IPC_PIPE") {
        Ok(v) if v.is_empty() => crate::ipc::DEFAULT_PIPE_NAME.to_string(),
        Ok(v) if v.starts_with(r"\\.\pipe\") => v,
        Ok(v) => format!(r"\\.\pipe\{v}"),
        Err(_) => crate::ipc::DEFAULT_PIPE_NAME.to_string(),
    }
}

/// Per-session pipe path so concurrent Runs do not share one name.
pub fn session_pipe_name(instance_id: &str) -> String {
    format!(r"\\.\pipe\envbox-runtime-{instance_id}")
}

/// Packaged roots derive this pipe path from their PID before requesting a Profile.
pub fn packaged_pipe_name(pid: u32) -> String {
    format!(r"\\.\pipe\envbox-runtime-pid-{pid}")
}

fn nudge_pipe(name: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };

        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            if let Ok(h) = CreateFileW(
                PCWSTR(wide.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            ) {
                let _ = CloseHandle(h);
            }
        }
    }
}

#[cfg(windows)]
fn serve_loop(
    table: SharedTable,
    stop: Arc<AtomicBool>,
    pipe_name: String,
    expected: Option<crate::service_start::TokenOwner>,
    mut ready: Option<std::sync::mpsc::SyncSender<std::io::Result<()>>>,
    #[cfg(test)] pause: Option<Arc<AcceptPause>>,
) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, HANDLE,
    };
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    let security = expected
        .as_ref()
        .map(|owner| crate::service_start::PipeSecurity::new(&owner.sid_text))
        .transpose();
    let security = match security {
        Ok(value) => value,
        Err(error) => {
            if let Some(tx) = ready.take() {
                let _ = tx.send(Err(error));
            }
            return;
        }
    };

    struct OwnedHandle(HANDLE);
    // Each connected pipe is moved to exactly one worker, which owns its
    // disconnect and close. No other thread accesses that handle.
    unsafe impl Send for OwnedHandle {}
    impl OwnedHandle {
        fn serve(
            self,
            table: &SharedTable,
            stop: &AtomicBool,
            expected: Option<&crate::service_start::TokenOwner>,
        ) {
            let _ = serve_connection(self.0, table, stop, expected);
        }
    }
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = DisconnectNamedPipe(self.0);
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    // Keep one accept loop so stop needs only one wakeup. Connected clients
    // run independently: an idle or slow client must not block new bootstrap
    // connections. The pipe instance limit also bounds live worker threads;
    // there is no unbounded queue of accepted clients.
    std::thread::scope(|scope| {
        let mut workers: Vec<std::thread::ScopedJoinHandle<'_, ()>> = Vec::new();
        #[cfg(test)]
        let mut iteration = 0;
        while !stop.load(Ordering::SeqCst) {
            let mut index = 0;
            while index < workers.len() {
                if workers[index].is_finished() {
                    let _ = workers.swap_remove(index).join();
                } else {
                    index += 1;
                }
            }
            if workers.len() == 8 {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            #[cfg(test)]
            {
                iteration += 1;
                if iteration == 2 {
                    if let Some(pause) = &pause {
                        let _ = pause.entered.send(());
                        if pause
                            .release
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            let name: Vec<u16> = std::ffi::OsStr::new(&pipe_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let first_instance = ready.is_some();
            let open_mode = if first_instance {
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                PIPE_ACCESS_DUPLEX
            };
            let raw = unsafe {
                CreateNamedPipeW(
                    PCWSTR(name.as_ptr()),
                    open_mode,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    8,
                    8192,
                    8192,
                    0,
                    security.as_ref().map(|sd| &sd.attributes as *const _),
                )
            };
            if raw.is_invalid() {
                let error = unsafe { GetLastError() };
                if first_instance && error == ERROR_ACCESS_DENIED {
                    if let Some(tx) = ready.take() {
                        let _ = tx.send(Err(std::io::Error::from_raw_os_error(error.0 as i32)));
                    }
                    return;
                }
                // Preserve the existing retry behavior for a transient bind
                // failure. start_on will stop the loop if the first instance does
                // not become available within its bounded readiness window.
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            let pipe = OwnedHandle(raw);
            #[cfg(test)]
            if iteration == 2 {
                if let Some(pause) = &pause {
                    let _ = pause.published.send(());
                }
            }
            if let Some(tx) = ready.take() {
                let _ = tx.send(Ok(()));
            }

            // stop's nudge may run after the loop condition but before this
            // instance exists. Once published, a later nudge can connect; an
            // earlier one is covered by this check before the blocking accept.
            if stop.load(Ordering::SeqCst) {
                return;
            }

            match unsafe { ConnectNamedPipe(pipe.0, None) } {
                Ok(()) => {}
                Err(_) => {
                    let code = unsafe { GetLastError() };
                    if code != ERROR_PIPE_CONNECTED {
                        drop(pipe);
                        continue;
                    }
                }
            }

            if stop.load(Ordering::SeqCst) {
                return;
            }
            let table = &table;
            let stop = &stop;
            let expected = expected.as_ref();
            match std::thread::Builder::new()
                .name("envbox-ipc-client".into())
                .spawn_scoped(scope, move || pipe.serve(table, stop, expected))
            {
                Ok(worker) => workers.push(worker),
                Err(_) => {
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
            }
        }
        // Joining explicitly also consumes any worker panic without making
        // shutdown panic. Early returns are joined by the scope itself.
        for worker in workers {
            let _ = worker.join();
        }
    });
}

#[cfg(windows)]
fn serve_connection(
    pipe: windows::Win32::Foundation::HANDLE,
    table: &SharedTable,
    stop: &AtomicBool,
    expected: Option<&crate::service_start::TokenOwner>,
) -> std::io::Result<()> {
    use windows::Win32::Foundation::{GetLastError, ERROR_NO_DATA};
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
    use windows::Win32::System::Pipes::{
        GetNamedPipeClientProcessId, SetNamedPipeHandleState, PIPE_NOWAIT,
    };

    fn write_reply(
        pipe: windows::Win32::Foundation::HANDLE,
        stop: &AtomicBool,
        reply: IpcMessage,
    ) -> std::io::Result<()> {
        let mut out = reply.encode_line().into_bytes();
        out.push(b'\n');
        // Message handling can wait for the registry lock or validate files.
        // Give its response a fresh I/O budget instead of silently discarding
        // it when the preceding read deadline expired during processing.
        let deadline = std::time::Instant::now() + Duration::from_millis(750);
        let mut offset = 0;
        while offset < out.len() {
            if stop.load(Ordering::SeqCst) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "IPC broker stopping",
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "IPC response write timed out",
                ));
            }
            let mut written = 0u32;
            unsafe { WriteFile(pipe, Some(&out[offset..]), Some(&mut written), None) }?;
            offset += written as usize;
            if written == 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }

    let mut client_pid = 0;
    unsafe { GetNamedPipeClientProcessId(pipe, &mut client_pid) }?;
    let client = authenticate_process(client_pid, expected)?;
    // Bounded, nonblocking connection I/O: a client that keeps a pipe open
    // without reading/writing must not hang broker teardown or bootstrap.
    let mode = PIPE_NOWAIT;
    unsafe { SetNamedPipeHandleState(pipe, Some(&mode), None, None) }?;

    let mut buf = [0u8; 8192];
    let mut acc = Vec::new();
    let mut deadline = std::time::Instant::now() + Duration::from_millis(750);
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut read = 0u32;
        let ok = unsafe { ReadFile(pipe, Some(&mut buf), Some(&mut read), None) };
        if ok.is_err()
            && unsafe { GetLastError() } == ERROR_NO_DATA
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        if ok.is_err() || read == 0 {
            return Ok(());
        }
        acc.extend_from_slice(&buf[..read as usize]);
        while let Some(pos) = acc.iter().position(|&b| b == b'\n') {
            if stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            if pos > crate::ipc::IPC_IDENTITY_MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "IPC line exceeds limit",
                ));
            }
            let line_bytes: Vec<u8> = acc.drain(..=pos).collect();
            let line = std::str::from_utf8(&line_bytes)
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid IPC UTF-8")
                })?
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if line.is_empty() {
                continue;
            }
            let Ok(msg) = IpcMessage::decode_line(&line) else {
                write_reply(pipe, stop, protocol_denied("malformed_message"))?;
                deadline = std::time::Instant::now() + Duration::from_millis(750);
                continue;
            };
            // Re-check the process generation on every message. A cached PID
            // alone must not authorize a newly created process after reuse.
            let reply = {
                let mut registry = table.lock().unwrap();
                // A worker may have waited behind an expensive identity check.
                // Do not start queued work after shutdown has been requested.
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if process_creation_time(client_pid) == Some(client.creation_time) {
                    registry.handle_client(&client, &msg)
                } else {
                    Some(protocol_denied("client_generation_changed"))
                }
            };
            if let Some(rep) = reply {
                write_reply(pipe, stop, rep)?;
            }
            deadline = std::time::Instant::now() + Duration::from_millis(750);
        }
        if acc.len() > crate::ipc::IPC_IDENTITY_MAX_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IPC line exceeds limit",
            ));
        }
    }
}

#[cfg(not(windows))]
fn serve_loop(
    _table: SharedTable,
    stop: Arc<AtomicBool>,
    _pipe_name: String,
    _expected: Option<()>,
    ready: Option<std::sync::mpsc::SyncSender<std::io::Result<()>>>,
    #[cfg(test)] _pause: Option<Arc<AcceptPause>>,
) {
    if let Some(tx) = ready {
        let _ = tx.send(Ok(()));
    }
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn serve_connection(_pipe: (), _table: &SharedTable, _stop: &AtomicBool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::FakeBroker;

    #[cfg(windows)]
    #[test]
    fn stop_before_next_pipe_publication_does_not_lose_wakeup() {
        let name = session_pipe_name(&uuid::Uuid::new_v4().to_string());
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let (published_tx, published_rx) = std::sync::mpsc::sync_channel(1);
        let (nudged_tx, nudged_rx) = std::sync::mpsc::channel();
        let pause = Arc::new(AcceptPause {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            published: published_tx,
            nudged: nudged_tx,
        });
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = std::sync::mpsc::sync_channel(1);
        let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
        let worker_name = name.clone();
        let worker = std::thread::spawn(move || {
            let table = Arc::new(Mutex::new(SessionTable::new()));
            let mut broker =
                HostBroker::start_on_impl(table, worker_name, None, Some(pause)).unwrap();
            started_tx.send(broker.stop.clone()).unwrap();
            stop_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            broker.stop();
            done_tx.send(()).unwrap();
        });
        let stopped = started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        nudge_pipe(&name);
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second accept reached unpublished boundary");
        stop_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !stopped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(stopped.load(Ordering::SeqCst));
        // stop's only nudge must finish while no pipe exists. The barrier
        // gives the server no opportunity to publish during this interval.
        nudged_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stop nudge finished before publication");
        release_tx.send(()).unwrap();
        published_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("next pipe published");
        let completed = done_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        if !completed {
            // RED cleanup connects after publication, so the owned broker
            // can finish; never join its blocked accept without waking it.
            nudge_pipe(&name);
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("cleanup stop completed");
        }
        worker.join().unwrap();
        assert!(completed, "stop lost its wakeup before pipe publication");
    }

    #[test]
    fn shared_table_starts_and_stops() {
        let table: SharedTable = Arc::new(Mutex::new(SessionTable::new()));
        // On non-Windows this is a no-op loop; on Windows it binds the pipe.
        if let Ok(mut broker) = HostBroker::start(table.clone()) {
            let msg = IpcMessage::Hello {
                pid: 1,
                instance_id: "i".into(),
            };
            let _ = broker.table().lock().unwrap().handle(&msg);
            broker.stop();
        }
    }

    #[test]
    fn fake_broker_still_works() {
        let mut b = FakeBroker::new();
        assert!(b.send(IpcMessage::RuntimeReady { pid: 1 }).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn start_returns_after_pipe_instance_exists() {
        use std::os::windows::ffi::OsStrExt;
        use uuid::Uuid;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };

        let name = session_pipe_name(&Uuid::new_v4().to_string());
        let table: SharedTable = Arc::new(Mutex::new(SessionTable::new()));
        let mut broker = HostBroker::start_on(table, name.clone()).expect("broker ready");
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let client = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        }
        .expect("start_on must publish a connectable pipe");
        unsafe {
            let _ = CloseHandle(client);
        }
        broker.stop();
    }

    #[cfg(windows)]
    #[test]
    fn two_sessions_cannot_own_the_same_pipe() {
        use uuid::Uuid;

        let name = session_pipe_name(&Uuid::new_v4().to_string());
        let table = || Arc::new(Mutex::new(SessionTable::new()));
        let mut first = HostBroker::start_on(table(), name.clone()).expect("first owner");
        let second = HostBroker::start_on(table(), name.clone());
        assert_eq!(second.err().unwrap().kind(), std::io::ErrorKind::AddrInUse);
        first.stop();
        drop(first);
        let mut next = HostBroker::start_on(table(), name).expect("owner released");
        next.stop();
    }
}
