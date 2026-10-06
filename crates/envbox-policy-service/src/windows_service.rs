use crate::protocol::{self, LaunchRequest, SERVICE_SID};
use envbox_launcher::launcher::win::SafeHandle;
use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        Security::Authorization::*,
        Security::*,
        Storage::FileSystem::*,
        System::{
            Pipes::*,
            Services::*,
            SystemServices::{SE_GROUP_ENABLED, SE_GROUP_USE_FOR_DENY_ONLY},
            Threading::*,
            IO::*,
        },
    },
};

static STOP: AtomicBool = AtomicBool::new(false);
type Result<T> = std::result::Result<T, String>;
fn err(e: windows::core::Error) -> String {
    format!("{e} (HRESULT={:#x})", e.code().0)
}

/// Owns a referenced process and a primary token derived independently from
/// the peer. Neither can be supplied in the request.
pub struct AuthenticatedClient {
    pub(crate) process: SafeHandle,
    pub(crate) primary: SafeHandle,
    peer_pipe: SafeHandle,
    pub session: u32,
    pub creation_time: u64,
}
impl AuthenticatedClient {
    pub fn primary_token(&self) -> HANDLE {
        self.primary.0
    }
    pub fn is_alive(&self) -> bool {
        unsafe {
            WaitForSingleObject(self.process.0, 0) == WAIT_TIMEOUT
                && PeekNamedPipe(self.peer_pipe.0, None, 0, None, None, None).is_ok()
        }
    }
}

/// Clients must validate the referenced pipe server before sending any launch
/// material. Keeping this handle pins identity across PID reuse.
pub fn authenticate_server(pipe: HANDLE) -> Result<SafeHandle> {
    unsafe {
        let mut pid = 0;
        GetNamedPipeServerProcessId(pipe, &mut pid).map_err(err)?;
        let process = SafeHandle(
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
            .map_err(err)?,
        );
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token).map_err(err)?;
        let token = SafeHandle(token);
        validate_service_token(token.0)?;
        let mut now = 0;
        GetNamedPipeServerProcessId(pipe, &mut now).map_err(err)?;
        if now != pid || WaitForSingleObject(process.0, 0) != WAIT_TIMEOUT {
            return Err("service peer changed or exited".into());
        }
        Ok(process)
    }
}

fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<usize>> {
    unsafe {
        let mut length = 0;
        let _ = GetTokenInformation(token, class, None, 0, &mut length);
        if length == 0 || length > 65_536 {
            return Err("invalid token information size".into());
        }
        let mut data = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token,
            class,
            Some(data.as_mut_ptr().cast()),
            length,
            &mut length,
        )
        .map_err(err)?;
        Ok(data)
    }
}
pub(crate) fn sid_bytes(sid: PSID) -> Result<Vec<u8>> {
    unsafe {
        if !IsValidSid(sid).as_bool() {
            return Err("invalid SID".into());
        }
        let length = GetLengthSid(sid) as usize;
        if length > 68 {
            return Err("invalid SID length".into());
        }
        Ok(std::slice::from_raw_parts(sid.0.cast::<u8>(), length).to_vec())
    }
}
pub(crate) fn user(token: HANDLE) -> Result<Vec<u8>> {
    let data = token_info(token, TokenUser)?;
    sid_bytes(unsafe { (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid })
}
fn scalar(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<u32> {
    let data = token_info(token, class)?;
    Ok(unsafe { *data.as_ptr().cast::<u32>() })
}
pub(crate) fn auth_id(token: HANDLE) -> Result<(u32, i32)> {
    let data = token_info(token, TokenStatistics)?;
    let s = unsafe { &*data.as_ptr().cast::<TOKEN_STATISTICS>() };
    Ok((s.AuthenticationId.LowPart, s.AuthenticationId.HighPart))
}
fn groups(token: HANDLE) -> Result<Vec<(Vec<u8>, u32)>> {
    let data = token_info(token, TokenGroups)?;
    let g = unsafe { &*data.as_ptr().cast::<TOKEN_GROUPS>() };
    let count = g.GroupCount as usize;
    if count > 1024
        || std::mem::offset_of!(TOKEN_GROUPS, Groups)
            + count * std::mem::size_of::<SID_AND_ATTRIBUTES>()
            > data.len() * std::mem::size_of::<usize>()
    {
        return Err("invalid token group count".into());
    }
    unsafe { std::slice::from_raw_parts(g.Groups.as_ptr(), count) }
        .iter()
        .map(|g| Ok((sid_bytes(g.Sid)?, g.Attributes)))
        .collect()
}
fn validate_service_token(token: HANDLE) -> Result<()> {
    if user(token)? != [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0] || scalar(token, TokenSessionId)? != 0
    {
        return Err("service must run as session-zero LocalSystem".into());
    }
    let group = groups(token)?.into_iter().any(|(s, a)| {
        s == SERVICE_SID
            && a & SE_GROUP_ENABLED as u32 != 0
            && a & SE_GROUP_USE_FOR_DENY_ONLY as u32 == 0
    });
    if !group {
        return Err("dedicated enabled AuraPolicyService SID required".into());
    }
    Ok(())
}
pub fn validate_service_identity() -> Result<()> {
    unsafe {
        let mut h = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut h).map_err(err)?;
        let token = SafeHandle(h);
        validate_service_token(token.0)?;
        let mut thread = HANDLE::default();
        if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut thread).is_ok() {
            let _thread = SafeHandle(thread);
            return Err("service thread must not impersonate".into());
        }
        if GetLastError() != ERROR_NO_TOKEN {
            return Err("unable to establish non-impersonating service thread".into());
        }
        Ok(())
    }
}
struct Revert;
impl Drop for Revert {
    fn drop(&mut self) {
        unsafe {
            if RevertToSelf().is_err() {
                std::process::abort();
            }
        }
    }
}
struct PreauthenticatedClient {
    process: SafeHandle,
    sid: Vec<u8>,
    auth: (u32, i32),
    session: u32,
    creation_time: u64,
}
fn preauthenticate_client(
    pipe: HANDLE,
    approve: impl FnOnce(HANDLE) -> Result<()>,
) -> Result<PreauthenticatedClient> {
    unsafe {
        let mut pid = 0;
        GetNamedPipeClientProcessId(pipe, &mut pid).map_err(err)?;
        let process = SafeHandle(
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
            .map_err(err)?,
        );
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token).map_err(err)?;
        let token = SafeHandle(token);
        let mut created = FILETIME::default();
        let mut exited = created;
        let mut kernel = created;
        let mut usertime = created;
        GetProcessTimes(
            process.0,
            &mut created,
            &mut exited,
            &mut kernel,
            &mut usertime,
        )
        .map_err(err)?;
        let creation_time =
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let identity = PreauthenticatedClient {
            sid: user(token.0)?,
            auth: auth_id(token.0)?,
            session: scalar(token.0, TokenSessionId)?,
            process,
            creation_time,
        };
        approve(identity.process.0)?;
        if WaitForSingleObject(identity.process.0, 0) != WAIT_TIMEOUT {
            return Err("preauthenticated peer exited".into());
        }
        Ok(identity)
    }
}
impl PreauthenticatedClient {
    fn matches(&self, client: &AuthenticatedClient) -> Result<()> {
        unsafe {
            if GetProcessId(self.process.0) != GetProcessId(client.process.0)
                || WaitForSingleObject(self.process.0, 0) != WAIT_TIMEOUT
            {
                return Err("management peer changed after read".into());
            }
        }
        if self.creation_time != client.creation_time
            || self.sid != user(client.primary_token())?
            || self.auth != auth_id(client.primary_token())?
            || self.session != client.session
        {
            return Err("management token changed after read".into());
        }
        Ok(())
    }
}
pub fn authenticate_client(pipe: HANDLE) -> Result<AuthenticatedClient> {
    unsafe {
        let mut pid = 0;
        GetNamedPipeClientProcessId(pipe, &mut pid).map_err(err)?;
        let process = SafeHandle(
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
            .map_err(err)?,
        );
        let mut times = [FILETIME::default(); 4];
        GetProcessTimes(
            process.0,
            &mut times[0],
            &mut times[1],
            &mut times[2],
            &mut times[3],
        )
        .map_err(err)?;
        let creation_time =
            (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime);
        let mut h = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY | TOKEN_DUPLICATE, &mut h).map_err(err)?;
        let actual = SafeHandle(h);
        ImpersonateNamedPipeClient(pipe).map_err(err)?;
        let revert = Revert;
        OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut h).map_err(err)?;
        let peer = SafeHandle(h);
        let session = scalar(actual.0, TokenSessionId)?;
        let integrity = token_info(actual.0, TokenIntegrityLevel)?;
        let label = &*integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let sid = sid_bytes(label.Label.Sid)?;
        let level = u32::from_le_bytes(sid[sid.len() - 4..].try_into().unwrap());
        if session == 0
            || level < 0x2000
            || level >= 0x3000
            || scalar(actual.0, TokenIsAppContainer)? != 0
            || scalar(actual.0, TokenElevation)? != 0
        {
            return Err("ordinary medium-integrity interactive client required".into());
        }
        if user(actual.0)? != user(peer.0)?
            || session != scalar(peer.0, TokenSessionId)?
            || auth_id(actual.0)? != auth_id(peer.0)?
            || scalar(peer.0, TokenImpersonationLevel)? < SecurityImpersonation.0 as u32
            || scalar(peer.0, TokenIsAppContainer)? != 0
            || scalar(peer.0, TokenElevation)? != 0
            || scalar(peer.0, TokenHasRestrictions)? != scalar(actual.0, TokenHasRestrictions)?
            || groups(actual.0)? != groups(peer.0)?
            || sid_bytes(
                (&*token_info(peer.0, TokenIntegrityLevel)?
                    .as_ptr()
                    .cast::<TOKEN_MANDATORY_LABEL>())
                    .Label
                    .Sid,
            )? != sid
        {
            return Err("pipe impersonation differs from actual client token".into());
        }
        // Network logon tokens must not become an interactive launch authority.
        if groups(actual.0)?
            .iter()
            .any(|(s, _)| s == &[1, 1, 0, 0, 0, 0, 0, 5, 2, 0, 0, 0])
        {
            return Err("network logon client rejected".into());
        }
        if !groups(actual.0)?.iter().any(|(s, a)| {
            s == &[1, 1, 0, 0, 0, 0, 0, 5, 4, 0, 0, 0]
                && a & SE_GROUP_ENABLED as u32 != 0
                && a & SE_GROUP_USE_FOR_DENY_ONLY as u32 == 0
        }) {
            return Err("interactive logon client required".into());
        }
        let mut primary = HANDLE::default();
        DuplicateTokenEx(
            actual.0,
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
        .map_err(err)?;
        let primary = SafeHandle(primary);
        drop(revert);
        let mut retained_pipe = HANDLE::default();
        DuplicateHandle(
            GetCurrentProcess(),
            pipe,
            GetCurrentProcess(),
            &mut retained_pipe,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(err)?;
        let peer_pipe = SafeHandle(retained_pipe);
        let mut now = 0;
        GetNamedPipeClientProcessId(pipe, &mut now).map_err(err)?;
        if now != pid || WaitForSingleObject(process.0, 0) == WAIT_OBJECT_0 {
            return Err("client disconnected or exited during authentication".into());
        }
        Ok(AuthenticatedClient {
            process,
            primary,
            peer_pipe,
            session,
            creation_time,
        })
    }
}

pub struct DriverSession {
    handle: SafeHandle,
    generation: u64,
}
impl DriverSession {
    pub fn open() -> Result<Self> {
        validate_service_identity()?;
        unsafe {
            let handle = SafeHandle(
                CreateFileW(
                    w!(r"\\.\AuraPolicyPrototype"),
                    GENERIC_READ.0 | GENERIC_WRITE.0,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
                .map_err(err)?,
            );
            let mut bytes = [0u8; 8];
            let mut returned = 0;
            DeviceIoControl(
                handle.0,
                0x00226004,
                None,
                0,
                Some(bytes.as_mut_ptr().cast()),
                8,
                Some(&mut returned),
                None,
            )
            .map_err(err)?;
            let generation = u64::from_le_bytes(bytes);
            if returned != 8 || generation == 0 {
                return Err("invalid driver session reply".into());
            }
            Ok(Self { handle, generation })
        }
    }
    /// Called while the service-created target is still behind the launch gate.
    pub fn bind(
        &self,
        process: HANDLE,
        request: &LaunchRequest,
        snapshot: &envbox_core::RunSnapshot,
    ) -> Result<()> {
        validate_service_identity()?;
        let bytes = protocol::policy_wire(
            self.generation,
            process.0 as usize as u64,
            request,
            snapshot,
        )?;
        unsafe {
            let mut returned = 0;
            DeviceIoControl(
                self.handle.0,
                0x0022a008,
                Some(bytes.as_ptr().cast()),
                136,
                None,
                0,
                Some(&mut returned),
                None,
            )
            .map_err(err)?;
            if returned != 0 {
                return Err("unexpected driver apply reply".into());
            }
        }
        Ok(())
    }
}

pub fn dispatch() -> Result<()> {
    unsafe {
        let mut name: Vec<u16> = protocol::SERVICE_NAME
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: PWSTR(name.as_mut_ptr()),
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW::default(),
        ];
        StartServiceCtrlDispatcherW(table.as_ptr()).map_err(err)
    }
}
unsafe extern "system" fn control(
    code: u32,
    _event: u32,
    _data: *mut c_void,
    _context: *mut c_void,
) -> u32 {
    if code == SERVICE_CONTROL_STOP || code == SERVICE_CONTROL_SHUTDOWN {
        STOP.store(true, Ordering::Release);
    }
    0
}
unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let Ok(status) = RegisterServiceCtrlHandlerExW(w!("AuraPolicyService"), Some(control), None)
    else {
        return;
    };
    let mut state = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: SERVICE_START_PENDING,
        dwWaitHint: 5000,
        ..Default::default()
    };
    let _ = SetServiceStatus(status, &state);
    let result = validate_service_identity().and_then(|_| serve(status, &mut state));
    state.dwCurrentState = SERVICE_STOPPED;
    state.dwControlsAccepted = 0;
    state.dwWin32ExitCode = if result.is_ok() {
        0
    } else {
        ERROR_SERVICE_SPECIFIC_ERROR.0
    };
    state.dwServiceSpecificExitCode = u32::from(result.is_err());
    let _ = SetServiceStatus(status, &state);
    if let Err(e) = result {
        eprintln!("AuraPolicyService stopped: {e}");
    }
}

fn serve(status: SERVICE_STATUS_HANDLE, state: &mut SERVICE_STATUS) -> Result<()> {
    let bundle = crate::bundle::InstalledBundle::from_service_executable()?;
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w!("D:P(A;;GA;;;SY)(A;;0x0012019b;;;AU)S:(ML;;NW;;;ME)"),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(err)?;
        struct Descriptor(PSECURITY_DESCRIPTOR);
        impl Drop for Descriptor {
            fn drop(&mut self) {
                unsafe {
                    let _ = LocalFree(HLOCAL(self.0 .0));
                }
            }
        }
        let _descriptor = Descriptor(descriptor);
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: BOOL(0),
        };
        let name: Vec<u16> = protocol::PIPE_NAME.encode_utf16().chain(Some(0)).collect();
        let pipe = SafeHandle(CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_FRAME as u32,
            MAX_FRAME as u32,
            0,
            Some(&sa),
        ));
        if pipe.0.is_invalid() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        state.dwCurrentState = SERVICE_RUNNING;
        state.dwControlsAccepted = SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN;
        state.dwWaitHint = 0;
        SetServiceStatus(status, state).map_err(err)?;
        while !STOP.load(Ordering::Acquire) {
            let connected =
                ConnectNamedPipe(pipe.0, None).is_ok() || GetLastError() == ERROR_PIPE_CONNECTED;
            if !connected {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            // Only a preapproved real manager may consume the bounded read
            // window. Pipe impersonation still waits for the first message.
            let preauth = preauthenticate_client(pipe.0, |process| {
                bundle.approve_manager(process)?;
                crate::prototype::reject_loaded_runtime(process)
            });
            if preauth.is_err() {
                let _ = DisconnectNamedPipe(pipe.0);
                continue;
            }
            let preauth = preauth?;
            let response = read_request(pipe.0).and_then(|bytes| {
                let client = authenticate_client(pipe.0)?;
                preauth.matches(&client)?;
                let command = protocol::decode_command(&bytes)?;
                handle_command(&client, &command)
            });
            let payload = serde_json::to_vec(
                &serde_json::json!({"version":1,"ok":response.is_ok(),"error":response.err()}),
            )
            .map_err(|e| e.to_string())?;
            let mut written = 0;
            let _ = WriteFile(pipe.0, Some(&payload), Some(&mut written), None);
            let _ = DisconnectNamedPipe(pipe.0);
        }
        Ok(())
    }
}
use protocol::MAX_FRAME;
fn read_request(pipe: HANDLE) -> Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut bytes = vec![0u8; MAX_FRAME + 1];
    loop {
        if STOP.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("request deadline or service shutdown".into());
        }
        unsafe {
            let mut read = 0;
            if ReadFile(pipe, Some(&mut bytes), Some(&mut read), None).is_ok() {
                if read == 0 || read as usize > MAX_FRAME {
                    return Err("invalid request frame".into());
                }
                bytes.truncate(read as usize);
                return Ok(bytes);
            }
            if GetLastError() != ERROR_NO_DATA {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn handle_command(_client: &AuthenticatedClient, _command: &protocol::Command) -> Result<()> {
    // Never promote the unqualified source prototype via configuration/env.
    Err("kernel backend is not qualified for Container launch".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_process_cannot_open_privileged_driver_session() {
        let error = DriverSession::open()
            .err()
            .expect("test runner must not be AuraPolicyService");
        assert!(
            error.contains("LocalSystem") || error.contains("dedicated enabled"),
            "{error}"
        );
        assert!(crate::prototype::PrototypeEngine::prepare().is_err());
    }
    #[test]
    fn actual_local_pipe_spoof_server_is_rejected_before_payload() {
        unsafe {
            let name = format!(r"\\.\pipe\AuraServiceSpoofTest-{}", uuid::Uuid::new_v4());
            let text: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let server = SafeHandle(CreateNamedPipeW(
                PCWSTR(text.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                1024,
                1024,
                0,
                None,
            ));
            assert!(!server.0.is_invalid());
            let client = SafeHandle(
                CreateFileW(
                    PCWSTR(text.as_ptr()),
                    0x0012019b,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
                .unwrap(),
            );
            let error = authenticate_server(client.0).unwrap_err();
            assert!(
                error.contains("LocalSystem") || error.contains("dedicated enabled"),
                "{error}"
            );
        }
    }
    #[test]
    fn unauthorized_silent_peer_is_rejected_without_consuming_read_deadline() {
        unsafe {
            let name = format!(r"\\.\pipe\AuraSilentPeerTest-{}", uuid::Uuid::new_v4());
            let text: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let server = SafeHandle(CreateNamedPipeW(
                PCWSTR(text.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                1024,
                1024,
                0,
                None,
            ));
            assert!(!server.0.is_invalid());
            let _client = SafeHandle(
                CreateFileW(
                    PCWSTR(text.as_ptr()),
                    0x0012019b,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
                .unwrap(),
            );
            let started = Instant::now();
            let result = preauthenticate_client(server.0, |process| {
                let actual = crate::bundle::management_image_path(process)?;
                if actual.file_name().is_some_and(|n| {
                    n.to_string_lossy().eq_ignore_ascii_case("aura.exe")
                        || n.to_string_lossy().eq_ignore_ascii_case("envbox.exe")
                }) {
                    return Ok(());
                }
                Err("unapproved management executable".into())
            });
            assert!(matches!(result,Err(ref e) if e.contains("unapproved")));
            DisconnectNamedPipe(server.0).unwrap();
            assert!(started.elapsed() < Duration::from_secs(2));
            println!(
                "silent_peer_rejected_before_read elapsed_ms={}",
                started.elapsed().as_millis()
            );
        }
    }
}
