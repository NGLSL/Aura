//! Trusted service startup seam. This does not enable Container or install a service.
use crate::{SessionError, SessionHandle, SessionStartRequest};
use std::{cell::Cell, collections::HashMap, path::PathBuf};
use windows::Win32::{Foundation::HANDLE, Security::*, System::Threading::*};

pub use crate::service_bundle::{
    lease_protected_path, validate_service_input_path, TrustedRuntimeBundle,
};

pub struct ServiceStartOptions<'a> {
    /// Borrowed validated primary token; caller retains ownership throughout startup.
    pub primary_token: HANDLE,
    /// Validated installed pair, whose file and ancestor leases remain held.
    pub runtime_bundle: &'a TrustedRuntimeBundle,
    pub before_resume: &'a mut dyn FnMut(HANDLE, u32, u64) -> Result<(), String>,
}

pub fn start_session_as_user(
    req: SessionStartRequest,
    instance_id: uuid::Uuid,
    options: ServiceStartOptions<'_>,
) -> Result<SessionHandle, SessionError> {
    if req.profile.is_none()
        || !matches!(&req.launch, envbox_core::LaunchTarget::Executable { path } if path.is_absolute())
    {
        return Err(failure(
            "service startup requires Profile and absolute ordinary executable",
        ));
    }
    if let envbox_core::LaunchTarget::Executable { path } = &req.launch {
        crate::service_bundle::validate_service_input_path(path).map_err(failure)?;
    }
    if let Some(cwd) = &req.working_directory {
        crate::service_bundle::validate_service_input_path(cwd).map_err(failure)?;
    }
    if req.arguments.iter().any(|arg| arg.contains('\0')) {
        return Err(failure("NUL in service arguments"));
    }
    if req.profile.as_ref().is_some_and(|profile| {
        profile
            .environment
            .keys()
            .any(|key| key.to_ascii_uppercase().starts_with("ENVBOX_"))
    }) {
        return Err(failure(
            "service Profile cannot override reserved ENVBOX controls",
        ));
    }
    TokenOwner::from_token(options.primary_token)?;
    let _target_lease = precheck(options.primary_token, &req)?;
    crate::session::start_session_with_options(
        req,
        Some(instance_id),
        None,
        false,
        true,
        Some(options),
    )
}
fn failure(message: impl Into<String>) -> SessionError {
    SessionError::Unsupported(message.into())
}

#[derive(Clone)]
pub(crate) struct TokenOwner {
    pub sid: Vec<u8>,
    pub sid_text: String,
    pub integrity: u32,
    pub session: u32,
}
unsafe fn information(
    token: HANDLE,
    class: TOKEN_INFORMATION_CLASS,
) -> std::io::Result<Vec<usize>> {
    let mut size = 0;
    let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut size) };
    if size == 0 || size > 65536 {
        return Err(std::io::Error::last_os_error());
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
impl TokenOwner {
    pub(crate) fn from_token(token: HANDLE) -> Result<Self, SessionError> {
        Self::read(token, true).map_err(|e| failure(format!("user token: {e}")))
    }
    pub(crate) fn read(token: HANDLE, primary: bool) -> std::io::Result<Self> {
        unsafe {
            if primary {
                let kind = information(token, TokenType)?;
                if *(kind.as_ptr().cast::<TOKEN_TYPE>()) != TokenPrimary {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "primary token required",
                    ));
                }
            }
            let user = information(token, TokenUser)?;
            let sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            let length = GetLengthSid(sid) as usize;
            if length == 0 || length > 1024 || !IsValidSid(sid).as_bool() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "invalid user SID",
                ));
            }
            let bytes = std::slice::from_raw_parts(sid.0.cast::<u8>(), length).to_vec();
            // String SID is produced from OS bytes, never from a request string.
            let mut text = windows::core::PWSTR::null();
            if ConvertSidToStringSidW(sid.0, &mut text.0) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let sid_text = text.to_string().map_err(std::io::Error::other);
            windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(
                text.0.cast(),
            ));
            let integrity = information(token, TokenIntegrityLevel)?;
            let label = (*(integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()))
                .Label
                .Sid;
            let count = *GetSidSubAuthorityCount(label);
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "invalid integrity SID",
                ));
            }
            let level = *GetSidSubAuthority(label, u32::from(count - 1));
            let session = information(token, TokenSessionId)?;
            Ok(Self {
                sid: bytes,
                sid_text: sid_text?,
                integrity: level,
                session: *(session.as_ptr().cast::<u32>()),
            })
        }
    }
    pub(crate) fn admits(&self, peer: &Self) -> bool {
        self.sid == peer.sid && self.integrity == peer.integrity && self.session == peer.session
    }
}

pub(crate) fn environment(token: HANDLE) -> Result<HashMap<String, String>, SessionError> {
    TokenOwner::from_token(token)?;
    unsafe {
        let mut raw = std::ptr::null_mut();
        if CreateEnvironmentBlock(&mut raw, token.0, 0) == 0 {
            return Err(failure(format!(
                "CreateEnvironmentBlock: {}",
                crate::launcher::win::last_error()
            )));
        }
        struct Block(*mut core::ffi::c_void);
        impl Drop for Block {
            fn drop(&mut self) {
                unsafe {
                    DestroyEnvironmentBlock(self.0);
                }
            }
        }
        let _block = Block(raw);
        let mut result = HashMap::new();
        let mut cursor = raw.cast::<u16>();
        let mut total = 0;
        while *cursor != 0 {
            let mut len = 0;
            while *cursor.add(len) != 0 {
                len += 1;
                total += 1;
                if total > 1_048_576 {
                    return Err(failure("user environment exceeds bound"));
                }
            }
            let entry = String::from_utf16(std::slice::from_raw_parts(cursor, len))
                .map_err(|_| failure("invalid user environment"))?;
            if let Some((key, value)) = entry.split_once('=') {
                if !key.is_empty() && !key.to_ascii_uppercase().starts_with("ENVBOX_") {
                    result.insert(key.into(), value.into());
                }
            }
            cursor = cursor.add(len + 1);
        }
        Ok(result)
    }
}
fn precheck(token: HANDLE, req: &SessionStartRequest) -> Result<std::fs::File, SessionError> {
    unsafe {
        ImpersonateLoggedOnUser(token)
            .map_err(|e| failure(format!("file precheck impersonation: {e}")))?;
    }
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            if unsafe { RevertToSelf() }.is_err() {
                std::process::abort();
            }
        }
    }
    let _revert = Revert;
    let envbox_core::LaunchTarget::Executable { path } = &req.launch else {
        unreachable!()
    };
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    // User READ + EXECUTE permission check; pin the exact file against writes
    // and replacement throughout creation and startup confirmation.
    let lease = std::fs::OpenOptions::new()
        .access_mode(0xA000_0000) // GENERIC_READ | GENERIC_EXECUTE
        .share_mode(1) // FILE_SHARE_READ
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(path)
        .map_err(|e| failure(format!("user cannot read/execute target: {e}")))?;
    if lease
        .metadata()
        .map_err(|e| failure(e.to_string()))?
        .file_attributes()
        & 0x400
        != 0
    {
        return Err(failure("service target cannot be a reparse point"));
    }
    if let Some(cwd) = &req.working_directory {
        std::fs::read_dir(cwd)
            .map_err(|e| failure(format!("user cannot access working directory: {e}")))?;
    }
    Ok(lease)
}
pub(crate) fn runtime_for_arch(
    options: &ServiceStartOptions<'_>,
    arch: crate::PeArch,
) -> Result<PathBuf, SessionError> {
    let native = if cfg!(target_pointer_width = "64") {
        crate::PeArch::X64
    } else {
        crate::PeArch::X86
    };
    if arch != native {
        return Err(failure(
            "trusted service cross-bitness helper creation is not qualified",
        ));
    }
    let path = if arch == crate::PeArch::X64 {
        options.runtime_bundle.runtime64()
    } else {
        options.runtime_bundle.runtime32()
    };
    if !path.is_absolute() || !path.is_file() {
        return Err(failure("approved installed Runtime missing"));
    }
    if crate::injection::pe_arch(path).map_err(|e| failure(e.to_string()))? != arch {
        return Err(failure("approved Runtime architecture mismatch"));
    }
    crate::recovery::validate_trusted_service_bootstrap(path)
        .map_err(|e| failure(e.to_string()))?;
    Ok(path.to_path_buf())
}
thread_local! { static CREATION_TOKEN: Cell<isize> = const { Cell::new(0) }; }
pub(crate) struct CreationTokenGuard(std::marker::PhantomData<std::rc::Rc<()>>);
impl CreationTokenGuard {
    pub(crate) fn enter(token: HANDLE) -> Result<Self, SessionError> {
        CREATION_TOKEN.with(|slot| {
            if slot.get() != 0 || token.is_invalid() {
                return Err(failure("invalid/reentrant service creation token"));
            }
            slot.set(token.0 as isize);
            Ok(Self(std::marker::PhantomData))
        })
    }
}
impl Drop for CreationTokenGuard {
    fn drop(&mut self) {
        CREATION_TOKEN.with(|slot| slot.set(0));
    }
}
pub(crate) fn creation_routine() -> *const core::ffi::c_void {
    CREATION_TOKEN.with(|slot| {
        if slot.get() == 0 {
            std::ptr::null()
        } else {
            create_as_user as *const core::ffi::c_void
        }
    })
}
unsafe extern "system" fn create_as_user(
    application: windows::core::PCWSTR,
    command: windows::core::PWSTR,
    process_attr: *const core::ffi::c_void,
    thread_attr: *const core::ffi::c_void,
    inherit: i32,
    flags: u32,
    environment: *const core::ffi::c_void,
    cwd: windows::core::PCWSTR,
    startup: *mut STARTUPINFOW,
    pi: *mut PROCESS_INFORMATION,
) -> i32 {
    let token = CREATION_TOKEN.with(|slot| slot.get());
    if token == 0 {
        unsafe {
            windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(6));
        }
        return 0;
    }
    // Explicit interactive desktop; never inherit the LocalSystem service desktop.
    // The trusted caller must supply an authenticated interactive-session token.
    unsafe {
        (*startup).lpDesktop =
            windows::core::PWSTR(windows::core::w!("winsta0\\default").0 as *mut _);
        CreateProcessAsUserW(
            HANDLE(token as *mut _),
            application,
            command,
            if process_attr.is_null() {
                None
            } else {
                Some(process_attr.cast())
            },
            if thread_attr.is_null() {
                None
            } else {
                Some(thread_attr.cast())
            },
            inherit != 0,
            PROCESS_CREATION_FLAGS(flags),
            Some(environment),
            cwd,
            startup,
            pi,
        )
        .is_ok() as i32
    }
}
#[link(name = "userenv")]
extern "system" {
    fn CreateEnvironmentBlock(
        block: *mut *mut core::ffi::c_void,
        token: *mut core::ffi::c_void,
        inherit: i32,
    ) -> i32;
    fn DestroyEnvironmentBlock(block: *mut core::ffi::c_void) -> i32;
}
#[link(name = "advapi32")]
extern "system" {
    fn ConvertSidToStringSidW(sid: *mut core::ffi::c_void, text: *mut *mut u16) -> i32;
}

pub(crate) struct PipeSecurity {
    pub attributes: SECURITY_ATTRIBUTES,
    descriptor: *mut core::ffi::c_void,
}
impl PipeSecurity {
    pub(crate) fn new(sid: &str) -> std::io::Result<Self> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;S-1-5-80-3820527054-1736232212-1554131609-2082016452-471345742)(A;;0x0012019b;;;{sid})S:(ML;;NW;;;ME)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = std::ptr::null_mut();
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self {
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: false.into(),
            },
            descriptor,
        })
    }
}
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(
                self.descriptor,
            ));
        }
    }
}
#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut *mut core::ffi::c_void,
        size: *mut u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn token() -> crate::launcher::win::SafeHandle {
        let mut raw = HANDLE::default();
        unsafe {
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw).unwrap();
        }
        crate::launcher::win::SafeHandle(raw)
    }
    #[test]
    fn owner_is_actual_token_and_session_integrity_are_exact() {
        let token = token();
        let owner = TokenOwner::from_token(token.0).unwrap();
        assert!(!owner.sid.is_empty());
        assert!(owner.sid_text.starts_with("S-1-"));
        assert!(owner.admits(&owner));
        let mut peer = owner.clone();
        peer.session ^= 1;
        assert!(!owner.admits(&peer));
        peer = owner.clone();
        peer.integrity += 1;
        assert!(!owner.admits(&peer));
        peer = owner.clone();
        peer.sid.push(1);
        assert!(!owner.admits(&peer));
        let security = PipeSecurity::new(&owner.sid_text).unwrap();
        assert!(!security.attributes.lpSecurityDescriptor.is_null());
    }
    #[test]
    fn user_pipe_rights_allow_connection_but_deny_additional_server_instance() {
        let token = token();
        let owner = TokenOwner::from_token(token.0).unwrap();
        let table = std::sync::Arc::new(std::sync::Mutex::new(crate::SessionTable::new()));
        let name = crate::session_pipe_name(&uuid::Uuid::new_v4().to_string());
        let mut broker =
            crate::HostBroker::start_on_owned(table, name.clone(), Some(owner)).unwrap();
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let fake = unsafe {
            windows::Win32::System::Pipes::CreateNamedPipeW(
                windows::core::PCWSTR(wide.as_ptr()),
                windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX,
                windows::Win32::System::Pipes::PIPE_TYPE_BYTE,
                8,
                8192,
                8192,
                0,
                None,
            )
        };
        let error = unsafe { windows::Win32::Foundation::GetLastError() };
        if !fake.is_invalid() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(fake);
            }
        }
        assert!(fake.is_invalid());
        assert_eq!(error, windows::Win32::Foundation::ERROR_ACCESS_DENIED);
        drop(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&name)
                .unwrap(),
        );
        broker.stop();
    }

    #[test]
    fn user_target_read_access_does_not_imply_execute_access() {
        let token = token();
        let owner = TokenOwner::from_token(token.0).unwrap();
        let path = std::env::temp_dir().join(format!(
            "aura-service-no-execute-{}.exe",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"fixture").unwrap();
        struct Remove(std::path::PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _remove = Remove(path.clone());
        let sddl: Vec<u16> = format!(
            "D:P(D;;0x20;;;{})(A;;GA;;;{})",
            owner.sid_text, owner.sid_text
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let mut descriptor = std::ptr::null_mut();
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut()
                ),
                0
            );
        }
        let descriptor = PipeSecurity {
            attributes: SECURITY_ATTRIBUTES::default(),
            descriptor,
        };
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            assert_ne!(SetFileSecurityW(wide.as_ptr(), 4, descriptor.descriptor), 0);
        }
        assert!(std::fs::File::open(&path).is_ok());
        let req = SessionStartRequest {
            application_id: uuid::Uuid::nil(),
            launch: envbox_core::LaunchTarget::Executable { path },
            arguments: vec![],
            working_directory: None,
            profile: None,
            inherit_children: false,
            audit: false,
        };
        assert!(precheck(token.0, &req).is_err());
    }
    #[test]
    fn creation_token_is_thread_local_reentrant_guarded_and_cleared() {
        assert!(creation_routine().is_null());
        let token = token();
        let guard = CreationTokenGuard::enter(token.0).unwrap();
        assert!(!creation_routine().is_null());
        assert!(CreationTokenGuard::enter(token.0).is_err());
        assert!(std::thread::spawn(|| creation_routine().is_null())
            .join()
            .unwrap());
        drop(guard);
        assert!(creation_routine().is_null());
        assert!(CreationTokenGuard::enter(HANDLE::default()).is_err());
    }

    #[test]
    #[ignore = "requires approved native Runtime and no-CRT startup fixture in fresh host"]
    fn native_binding_precedes_entry_and_failure_leaves_no_marker() {
        use envbox_core::{
            DnsMode, DnsProfile, EnvironmentProfile, LaunchTarget, LocaleProfile, RegistryProfile,
            TimezoneProfile,
        };
        unsafe {
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            assert!(GetModuleHandleW(windows::core::w!("envbox-runtime64.dll")).is_err());
            assert!(GetModuleHandleW(windows::core::w!("envbox-runtime32.dll")).is_err());
        }
        println!(
            "service_fixture_pid={} runtime_modules=0",
            std::process::id()
        );
        let target = PathBuf::from(std::env::var("AURA_SERVICE_TARGET").unwrap());
        let dll = PathBuf::from(std::env::var("AURA_SERVICE_DLL").unwrap());
        let mut raw = HANDLE::default();
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY,
                &mut raw,
            )
            .unwrap();
        }
        let token = crate::launcher::win::SafeHandle(raw);
        for scenario in ["missing", "bind_failure", "untrusted_server"] {
            let instance = uuid::Uuid::new_v4();
            let marker = std::env::temp_dir().join(format!("aura-service-entry-{instance}.txt"));
            let mut profile = EnvironmentProfile {
                identity: Default::default(),
                id: uuid::Uuid::new_v4(),
                name: "trusted-service-fixture".into(),
                locale: LocaleProfile {
                    locale_name: "en-US".into(),
                    ui_language: "en-US".into(),
                    region: "US".into(),
                },
                timezone: TimezoneProfile {
                    windows_id: "Pacific Standard Time".into(),
                    iana_id: "America/Los_Angeles".into(),
                },
                dns: DnsProfile {
                    mode: DnsMode::Host,
                    servers: vec![],
                    ..Default::default()
                },
                environment: HashMap::new(),
                registry: RegistryProfile::default(),
                browser: Default::default(),
            };
            profile
                .environment
                .insert("AURA_GATE_MARKER".into(), marker.display().to_string());
            let req = SessionStartRequest {
                application_id: uuid::Uuid::nil(),
                launch: LaunchTarget::Executable {
                    path: target.clone(),
                },
                arguments: vec![],
                working_directory: target.parent().map(Path::to_path_buf),
                profile: Some(profile),
                inherit_children: false,
                audit: false,
            };
            let mut called = false;
            let mut binding = |handle: HANDLE, pid: u32, generation: u64| {
                called = true;
                assert!(!marker.exists());
                assert_eq!(unsafe { GetProcessId(handle) }, pid);
                assert!(generation > 0);
                if scenario == "bind_failure" {
                    Err("fixture bind denied".into())
                } else {
                    Ok(())
                }
            };
            let missing = dll.with_file_name("missing-approved.dll");
            let selected = if scenario == "missing" {
                &missing
            } else {
                &dll
            };
            // Test-only fixture bypasses installed ACL trust; no production constructor does.
            let bundle = TrustedRuntimeBundle::fixture(selected.clone(), selected.clone());
            let result = start_session_as_user(
                req,
                instance,
                ServiceStartOptions {
                    primary_token: token.0,
                    runtime_bundle: &bundle,
                    before_resume: &mut binding,
                },
            );
            assert!(result.is_err());
            assert_eq!(called, scenario != "missing");
            assert!(!marker.exists());
            println!("service_ordering scenario={scenario} passed");
        }
    }
    #[test]
    fn token_environment_contains_user_values_and_no_envbox_inheritance() {
        let token = token();
        let env = environment(token.0).unwrap();
        assert!(env
            .keys()
            .any(|key| key.eq_ignore_ascii_case("USERPROFILE")));
        assert!(!env
            .keys()
            .any(|key| key.to_ascii_uppercase().starts_with("ENVBOX_")));
        assert!(environment(HANDLE::default()).is_err());
    }
}

#[cfg(test)]
#[link(name = "advapi32")]
extern "system" {
    fn SetFileSecurityW(
        path: *const u16,
        information: u32,
        descriptor: *const core::ffi::c_void,
    ) -> i32;
}
