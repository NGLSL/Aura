//! Independent owner for GUI/CLI compatibility sessions. The startup pipe is
//! private to the launching process and its child; profile data never appears
//! in a command line or a temporary file. Runtime IPC keeps its existing owner
//! and authentication rules inside the helper until the process tree ends.

use crate::ipc::ObservedRuntimeIdentity;
use crate::job::InstanceJob;
use crate::launcher::win::SafeHandle;
use crate::process_tracker::ProcessTracker;
use crate::session::{SessionError, SessionHandle, SessionStartRequest};
use envbox_core::{EnvironmentSession, LaunchTarget, RuntimeInstance};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use uuid::Uuid;
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::*;

const VERSION: u32 = 1;
const MAX_FRAME: usize = 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize)]
struct Request {
    version: u32,
    request: SessionStartRequest,
    instance_id: Uuid,
    job_name: String,
    owns_job: bool,
    cleanup_job: bool,
    create_new_console: bool,
    inherit_console: bool,
}

#[derive(Serialize, Deserialize)]
struct Ready {
    session: EnvironmentSession,
    instance: RuntimeInstance,
    tracker: ProcessTracker,
    process_handle: u64,
    identity: Option<ObservedRuntimeIdentity>,
}

#[derive(Serialize, Deserialize)]
struct Reply {
    version: u32,
    result: Result<Ready, String>,
}

#[derive(Serialize, Deserialize)]
struct Acknowledgement {
    version: u32,
}

fn fail(error: impl std::fmt::Display) -> SessionError {
    SessionError::Unsupported(format!("session host: {error}"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn pause(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "session host startup deadline expired",
        ));
    }
    std::thread::sleep(Duration::from_millis(10));
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn frame_size(length: u32) -> io::Result<usize> {
    let length = length as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(invalid("invalid session host frame size"));
    }
    Ok(length)
}

fn read_exact(pipe: HANDLE, bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "startup read timeout",
            ));
        }
        let mut count = 0;
        let result = unsafe { ReadFile(pipe, Some(&mut bytes[offset..]), Some(&mut count), None) };
        offset += count as usize;
        if let Err(error) = result {
            let code = error.code().0 as u32 & 0xffff;
            if code != ERROR_NO_DATA.0 && code != ERROR_PIPE_LISTENING.0 {
                return Err(error.into());
            }
            // ERROR_NO_DATA also describes a disconnected nonblocking pipe.
            // Peek distinguishes that state from an empty, connected pipe.
            unsafe { PeekNamedPipe(pipe, None, 0, None, None, None) }?;
        }
        if count == 0 {
            pause(deadline)?;
        }
    }
    Ok(())
}

fn receive<T: DeserializeOwned>(pipe: HANDLE, deadline: Instant) -> io::Result<T> {
    let mut header = [0u8; 4];
    read_exact(pipe, &mut header, deadline)?;
    let length = frame_size(u32::from_le_bytes(header))?;
    let mut data = vec![0u8; length];
    read_exact(pipe, &mut data, deadline)?;
    serde_json::from_slice(&data).map_err(Into::into)
}

fn send<T: Serialize>(pipe: HANDLE, value: &T, deadline: Instant) -> io::Result<()> {
    let payload = serde_json::to_vec(value)?;
    frame_size(u32::try_from(payload.len()).map_err(|_| invalid("oversized startup frame"))?)?;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    let mut offset = 0;
    while offset < frame.len() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "startup write timeout",
            ));
        }
        let mut count = 0;
        // Small writes on a PIPE_NOWAIT byte pipe keep a full pipe from blocking
        // the launcher indefinitely. Zero-byte writes are retried to deadline.
        let limit = (offset + 4096).min(frame.len());
        unsafe { WriteFile(pipe, Some(&frame[offset..limit]), Some(&mut count), None) }?;
        offset += count as usize;
        if count == 0 {
            pause(deadline)?;
        }
    }
    Ok(())
}

fn verify_version(version: u32) -> io::Result<()> {
    if version != VERSION {
        return Err(invalid("unsupported session host protocol"));
    }
    Ok(())
}

fn kill_helper(child: &mut Child) {
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_secs(1);
    while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_helper(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
        return Err(invalid("invalid session helper executable size"));
    }
    Ok(bytes)
}

fn stage_helper(source: &Path) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let bytes = read_helper(source)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let directory = base.join("com.aura.envbox").join("session-host").join(hash);
    std::fs::create_dir_all(&directory)?;
    let destination = directory.join("envbox-broker.exe");
    if destination.exists() {
        if read_helper(&destination)? != bytes {
            return Err(invalid("cached session helper fingerprint mismatch"));
        }
        return Ok(destination);
    }
    let temporary = directory.join(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Windows rename never overwrites an existing, possibly running helper.
        // A concurrent publisher is accepted only after checking its content.
        if let Err(error) = std::fs::rename(&temporary, &destination) {
            if !destination.exists() {
                return Err(error);
            }
        }
        if read_helper(&destination)? != bytes {
            return Err(invalid("staged session helper fingerprint mismatch"));
        }
        Ok(destination)
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

/// Start a compatibility session owned by a separate broker process. Host-mode
/// launches stay local and do not allocate an unnecessary helper.
pub fn start(
    req: SessionStartRequest,
    requested_instance_id: Option<Uuid>,
    requested_job_name: Option<String>,
    create_new_console: bool,
) -> Result<SessionHandle, SessionError> {
    start_impl(
        req,
        requested_instance_id,
        requested_job_name,
        create_new_console,
        false,
    )
}

/// Windows Terminal's bridge owns a unique Job created by this Run. Unlike a
/// generic caller-provided tracking Job, that ownership authorizes startup
/// failure cleanup of the entire newly launched tree.
pub fn start_in_owned_job(
    req: SessionStartRequest,
    instance_id: Uuid,
    job_name: impl AsRef<str>,
) -> Result<SessionHandle, SessionError> {
    start_impl(
        req,
        Some(instance_id),
        Some(job_name.as_ref().to_string()),
        false,
        true,
    )
}

fn broker_source() -> Result<PathBuf, SessionError> {
    let current = std::env::current_exe().map_err(fail)?;
    let beside = current.with_file_name("envbox-broker.exe");
    if beside.is_file() {
        return Ok(beside);
    }
    // Cargo test executables live in target/<profile>/deps, while the helper
    // is a normal target/<profile> binary. Installed discovery stays sibling-only.
    if let Some(parent) = current
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "deps"))
    {
        if let Some(target) = parent.parent() {
            let broker = target.join("envbox-broker.exe");
            if broker.is_file() {
                return Ok(broker);
            }
        }
    }
    Err(fail(format!(
        "session helper not found: {}",
        beside.display()
    )))
}

fn start_impl(
    req: SessionStartRequest,
    requested_instance_id: Option<Uuid>,
    requested_job_name: Option<String>,
    create_new_console: bool,
    cleanup_owned_job: bool,
) -> Result<SessionHandle, SessionError> {
    if req.profile.is_none() {
        return crate::session::start_session_with_options(
            req,
            requested_instance_id,
            requested_job_name,
            create_new_console,
            false,
            None,
        );
    }
    let owns_job = requested_job_name.is_none();
    let may_cleanup_job = owns_job || cleanup_owned_job;
    let is_packaged = matches!(
        crate::package_discovery::normalize_launch_target(&req.launch),
        LaunchTarget::Packaged { .. }
    );
    let instance_id = requested_instance_id.unwrap_or_else(Uuid::new_v4);
    let job_name =
        requested_job_name.unwrap_or_else(|| format!(r"Local\AuraSessionHost-{}", Uuid::new_v4()));
    let job = if owns_job {
        InstanceJob::create_named_exclusive(&job_name)?
    } else {
        InstanceJob::open_named(&job_name)?
    };
    job.verify_tracking_limits()?;
    // Keep an independent ownership reference during the whole exchange, even
    // when the closure drops its proxy Job on a failed result.
    let cleanup_job = if may_cleanup_job && !is_packaged {
        Some(InstanceJob::open_named(&job_name)?)
    } else {
        None
    };
    let name = format!(r"\\.\pipe\aura-session-start-{}", Uuid::new_v4());
    let name_wide = wide(&name);
    let pipe = SafeHandle(unsafe {
        CreateNamedPipeW(
            PCWSTR(name_wide.as_ptr()),
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
        return Err(fail(io::Error::last_os_error()));
    }
    let executable = stage_helper(&broker_source()?).map_err(fail)?;
    // Resolve while current_exe still points at the installed/build directory.
    // Moving the host outside that directory must not change DLL discovery.
    let runtime_source = if is_packaged {
        crate::injection::resolve_runtime_dll()
    } else {
        let environment = crate::environment::build_environment_block(
            &std::env::vars().collect(),
            req.profile.as_ref(),
            instance_id,
            req.profile.as_ref().unwrap().id,
            req.inherit_children,
            req.audit,
        );
        let program =
            crate::launcher::activation_program(&req.launch, &req.arguments, &environment)?;
        crate::injection::resolve_runtime_dll_for_target(&program)
    }
    .map_err(fail)?;
    let runtime_source =
        crate::injection::stage_runtime_dll(&runtime_source, instance_id).map_err(fail)?;
    let mut command = Command::new(executable);
    command
        .arg("--session-host")
        .arg(&name)
        .arg(std::process::id().to_string());
    command.env("ENVBOX_RUNTIME_DLL", runtime_source);
    // Windows forcibly kills console members when the caller closes its
    // terminal. Keep the owner detached, attaching only for target activation.
    let inherit_console =
        !create_new_console && unsafe { windows::Win32::System::Console::GetConsoleCP() } != 0;
    command.creation_flags(CREATE_NO_WINDOW.0);
    let mut helper = command.spawn().map_err(fail)?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let result = (|| {
        loop {
            let connected = unsafe { ConnectNamedPipe(pipe.0, None) };
            if connected.is_ok() || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
                break;
            }
            if unsafe { GetLastError() } != ERROR_PIPE_LISTENING {
                return Err(fail(io::Error::last_os_error()));
            }
            if helper.try_wait().map_err(fail)?.is_some() {
                return Err(fail("helper exited before startup"));
            }
            pause(deadline).map_err(fail)?;
        }
        let mut peer = 0;
        unsafe { GetNamedPipeClientProcessId(pipe.0, &mut peer) }.map_err(fail)?;
        if peer != helper.id() {
            return Err(fail("startup client is not the spawned helper"));
        }
        send(
            pipe.0,
            &Request {
                version: VERSION,
                request: req,
                instance_id,
                job_name,
                owns_job,
                cleanup_job: may_cleanup_job,
                create_new_console,
                inherit_console,
            },
            deadline,
        )
        .map_err(fail)?;
        let reply: Reply = receive(pipe.0, deadline).map_err(fail)?;
        verify_version(reply.version).map_err(fail)?;
        let ready = reply.result.map_err(fail)?;
        // The helper retains its original root handle until our ACK. Duplicate
        // from that exact child, rather than reopening a possibly exited PID or
        // leaving a remote handle in the parent on an interrupted handoff.
        let mut process = HANDLE::default();
        unsafe {
            DuplicateHandle(
                HANDLE(helper.as_raw_handle()),
                HANDLE(ready.process_handle as usize as *mut _),
                GetCurrentProcess(),
                &mut process,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        }
        .map_err(fail)?;
        let process = SafeHandle(process);
        if process.0.is_invalid() || unsafe { GetProcessId(process.0) } != ready.instance.root_pid {
            return Err(fail("invalid retained startup process handle"));
        }
        send(pipe.0, &Acknowledgement { version: VERSION }, deadline).map_err(fail)?;
        let committed: Acknowledgement = receive(pipe.0, deadline).map_err(fail)?;
        verify_version(committed.version).map_err(fail)?;
        Ok(SessionHandle::from_host(
            ready.session,
            ready.instance,
            job,
            ready.tracker,
            process,
            ready.identity,
        ))
    })();
    if result.is_err() {
        // Only a freshly owned Win32 Job authorizes whole-tree cleanup. AUMID
        // activation can reuse an existing user process and must never kill it.
        kill_helper(&mut helper);
        if let Some(job) = cleanup_job {
            job.terminate()?;
        }
    }
    result
}

fn open_startup_pipe(name: &str, parent_pid: u32, deadline: Instant) -> io::Result<SafeHandle> {
    if !name.starts_with(r"\\.\pipe\aura-session-start-") {
        return Err(invalid("invalid startup pipe name"));
    }
    let name = wide(name);
    loop {
        match unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        } {
            Ok(handle) => {
                let pipe = SafeHandle(handle);
                let mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
                unsafe { SetNamedPipeHandleState(pipe.0, Some(&mode), None, None) }?;
                let mut actual_parent = 0;
                unsafe { GetNamedPipeServerProcessId(pipe.0, &mut actual_parent) }?;
                if actual_parent != parent_pid {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "startup server is not the launching parent",
                    ));
                }
                return Ok(pipe);
            }
            Err(error) => {
                let code = error.code().0 as u32 & 0xffff;
                if code != ERROR_PIPE_BUSY.0 && code != ERROR_FILE_NOT_FOUND.0 {
                    return Err(error.into());
                }
                pause(deadline)?;
            }
        }
    }
}

/// Internal `envbox-broker --session-host` entry point. No public management
/// endpoint is added: the parent PID and the spawned helper PID authenticate
/// the single bounded startup exchange in both directions.
pub fn run(name: &str, parent_pid: u32) -> Result<(), SessionError> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let pipe = open_startup_pipe(name, parent_pid, deadline).map_err(fail)?;
    let request: Request = receive(pipe.0, deadline).map_err(fail)?;
    verify_version(request.version).map_err(fail)?;
    let mut console = match ConsoleAttachment::attach(parent_pid, request.inherit_console) {
        Ok(console) => console,
        Err(error) => {
            let _ = send(
                pipe.0,
                &Reply {
                    version: VERSION,
                    result: Err(error.to_string()),
                },
                deadline,
            );
            return Err(fail(error));
        }
    };
    install_ctrlc_handler();
    let is_packaged = matches!(
        crate::package_discovery::normalize_launch_target(&request.request.launch),
        LaunchTarget::Packaged { .. }
    );
    let mut session = match crate::session::start_session_with_options_and_tracking(
        request.request,
        Some(request.instance_id),
        Some(request.job_name),
        request.create_new_console,
        false,
        None,
        request.owns_job && is_packaged,
    ) {
        Ok(session) => session,
        Err(error) => {
            let _ = send(
                pipe.0,
                &Reply {
                    version: VERSION,
                    result: Err(error.to_string()),
                },
                deadline,
            );
            return Err(error);
        }
    };
    let result = (|| {
        let process = session
            .process_handle()
            .ok_or_else(|| fail("missing retained root handle"))?;
        send(
            pipe.0,
            &Reply {
                version: VERSION,
                result: Ok(Ready {
                    session: session.session.clone(),
                    instance: session.instance.clone(),
                    tracker: session.tracker.clone(),
                    process_handle: process.0 as usize as u64,
                    identity: session.runtime_identity(),
                }),
            },
            deadline,
        )
        .map_err(fail)?;
        let acknowledgement: Acknowledgement = receive(pipe.0, deadline).map_err(fail)?;
        verify_version(acknowledgement.version).map_err(fail)?;
        // Commit proves the owner has already left the caller's console; the
        // caller can close its terminal immediately after a successful start.
        console.detach().map_err(fail)?;
        send(pipe.0, &Acknowledgement { version: VERSION }, deadline).map_err(fail)
    })();
    if let Err(error) = result {
        if !is_packaged {
            if request.cleanup_job {
                if let Some(job) = &session.job {
                    let _ = job.terminate();
                }
            } else if let Some(process) = session.process_handle() {
                unsafe {
                    let _ = TerminateProcess(process, 1);
                }
            }
        }
        return Err(error);
    }
    drop(pipe);
    drop(console);
    // Job accounting includes children even when the root already exited.
    // Package activation may not permit Job assignment; the authenticated
    // registry supplies generation-bound membership for that fallback.
    loop {
        let job_active = session.job.as_ref().is_some_and(|job| {
            job.stats()
                .map(|stats| stats.active_processes != 0)
                .unwrap_or(true)
        });
        let members = session.broker.as_ref().map(|broker| {
            broker
                .table()
                .lock()
                .ok()
                .map(|table| table.live_members(&session.instance.profile_id.to_string()))
        });
        // Unknown tracking state cannot prove that no targets remain.
        let registry_unknown = matches!(members, Some(None));
        let members = members.flatten().unwrap_or_default();
        let member_active = members.into_iter().any(|(pid, generation)| {
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )
            }
            .ok()
            .map(SafeHandle)
            .is_some_and(|process| {
                crate::ipc_server::process_creation_time(pid) == Some(generation)
                    && (unsafe { WaitForSingleObject(process.0, 0) }) == WAIT_TIMEOUT
            })
        });
        let root_active = session
            .process_handle()
            .is_some_and(|process| unsafe { WaitForSingleObject(process, 0) == WAIT_TIMEOUT });
        if !job_active && !member_active && !root_active && !registry_unknown {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // Explicitly retain the entire session until the last tracked process ends.
    if let Some(broker) = session.broker.as_mut() {
        broker.stop();
    }
    Ok(())
}

struct ConsoleAttachment(bool);

impl ConsoleAttachment {
    fn attach(parent_pid: u32, needed: bool) -> io::Result<Self> {
        use windows::Win32::System::Console::{
            AttachConsole, FreeConsole, GetConsoleCP, GetStdHandle, SetStdHandle, STD_ERROR_HANDLE,
            STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        if !needed {
            return Ok(Self(false));
        }
        // Preserve redirected standard handles across console initialization.
        let saved = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|kind| (kind, unsafe { GetStdHandle(kind) }.ok()));
        // CREATE_NO_WINDOW can still allocate a hidden console for a console
        // executable. Windows rejects AttachConsole while one is attached.
        if unsafe { GetConsoleCP() } != 0 {
            unsafe { FreeConsole() }.map_err(|error| {
                io::Error::other(format!("release helper console before activation: {error}"))
            })?;
        }
        unsafe { AttachConsole(parent_pid) }.map_err(|error| {
            io::Error::other(format!("attach launching console for activation: {error}"))
        })?;
        let attached = Self(true);
        for (kind, handle) in saved {
            if let Some(handle) = handle.filter(|handle| !handle.is_invalid()) {
                unsafe { SetStdHandle(kind, handle) }.map_err(|error| {
                    io::Error::other(format!("restore inherited standard handle: {error}"))
                })?;
            }
        }
        Ok(attached)
    }

    fn detach(&mut self) -> io::Result<()> {
        if self.0 {
            unsafe { windows::Win32::System::Console::FreeConsole() }.map_err(|error| {
                io::Error::other(format!(
                    "detach session owner before startup commit: {error}"
                ))
            })?;
            self.0 = false;
        }
        Ok(())
    }
}

impl Drop for ConsoleAttachment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                let _ = windows::Win32::System::Console::FreeConsole();
            }
        }
    }
}

fn install_ctrlc_handler() {
    use windows::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe extern "system" fn handler(_event: u32) -> BOOL {
        TRUE
    }
    // A real handler is used instead of the inheritable Ctrl-C-ignore flag so
    // newly activated console programs keep their normal signal behavior.
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(handler), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_frames_and_protocol_are_bounded() {
        assert!(frame_size(0).is_err());
        assert!(frame_size(MAX_FRAME as u32 + 1).is_err());
        assert_eq!(frame_size(MAX_FRAME as u32).unwrap(), MAX_FRAME);
        assert!(verify_version(VERSION + 1).is_err());
        assert!(verify_version(VERSION).is_ok());
    }

    #[test]
    fn helper_rejects_startup_pipe_owned_by_another_process() {
        let name = format!(r"\\.\pipe\aura-session-start-{}", Uuid::new_v4());
        let name_wide = wide(&name);
        let pipe = SafeHandle(unsafe {
            CreateNamedPipeW(
                PCWSTR(name_wide.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                None,
            )
        });
        assert_ne!(pipe.0, INVALID_HANDLE_VALUE);
        let result = open_startup_pipe(
            &name,
            std::process::id().wrapping_add(1),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    }
}
