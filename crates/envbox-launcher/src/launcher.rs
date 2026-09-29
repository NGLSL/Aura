//! LaunchRequest → LaunchedProcess with Detours Runtime injection (ticket 04).
//!
//! Sequence: resolve → env block → Job → DetourCreateProcessWithDllExW
//! (CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT) → assign Job → Resume.

use crate::command::{resolve_command, CommandError, ResolvedCommand};
use crate::environment::{build_environment_block, encode_environment_block};
use crate::job::{InstanceJob, JobError};
use envbox_core::{EnvironmentProfile, LaunchTarget};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Job(#[from] JobError),
    #[error("working directory does not exist: {0}")]
    WorkingDirectoryMissing(PathBuf),
    #[error("profile invalid: {0}")]
    InvalidProfile(String),
    #[error("failed to create process (GetLastError={code}): {message}")]
    CreateProcess { code: u32, message: String },
    #[error("comspec not set")]
    ComSpecMissing,
    #[error(transparent)]
    Inject(#[from] crate::injection::InjectError),
}

impl LaunchError {
    /// Win32/API failure carrying the saved GetLastError code.
    pub fn create_process(code: u32, message: impl Into<String>) -> Self {
        Self::CreateProcess {
            code,
            message: message.into(),
        }
    }

    /// Non-Win32 / policy rejection (no OS error code).
    pub fn create_process_msg(message: impl Into<String>) -> Self {
        Self::CreateProcess {
            code: 0,
            message: message.into(),
        }
    }
}

pub struct LaunchRequest {
    pub launch: LaunchTarget,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// `Some` = Environment Profile (inject Runtime). `None` = Host (no virtualization).
    pub profile: Option<EnvironmentProfile>,
    pub instance_id: Uuid,
    /// Child processes inherit the Environment Profile (Application.inherit_children).
    pub inherit_children: bool,
    /// Audit Mode (ticket 20): write JSONL audit trail for this instance.
    pub audit: bool,
}

pub struct LaunchedProcess {
    pub pid: u32,
    pub instance_id: Uuid,
    pub profile_id: Uuid,
    pub job: InstanceJob,
    #[cfg(windows)]
    process: win::SafeHandle,
    /// Primary thread handle (RAII; kept until process exit).
    #[cfg(windows)]
    #[allow(dead_code)]
    thread: win::SafeHandle,
    #[cfg(windows)]
    waited: bool,
}

#[cfg(windows)]
pub mod win {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};

    pub fn last_error() -> u32 {
        unsafe { GetLastError().0 }
    }

    /// RAII process/thread HANDLE. Not Clone — one owner closes it.
    #[derive(Debug)]
    pub struct SafeHandle(pub HANDLE);

    impl Drop for SafeHandle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }
}

impl LaunchedProcess {
    pub fn wait(&mut self) -> Result<std::process::ExitStatus, std::io::Error> {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            use windows::Win32::System::Threading::{
                GetExitCodeProcess, WaitForSingleObject, INFINITE,
            };

            unsafe {
                // WAIT_FAILED = 0xFFFFFFFF
                if WaitForSingleObject(self.process.0, INFINITE).0 == 0xFFFF_FFFF {
                    return Err(std::io::Error::last_os_error());
                }
                let mut code = 0u32;
                if GetExitCodeProcess(self.process.0, &mut code).is_err() {
                    return Err(std::io::Error::last_os_error());
                }
                self.waited = true;
                Ok(std::process::ExitStatus::from_raw(code))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = &self.waited;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "wait is windows-only",
            ))
        }
    }

    /// Explicitly stop the Process Tree Instance through its Job Object.
    pub fn stop(&mut self) -> Result<(), crate::job::JobError> {
        self.job.close()
    }

    pub fn job_stats(&self) -> Result<crate::job::JobStats, LaunchError> {
        Ok(self.job.stats()?)
    }
}

/// Join argv for display/edit (CommandLineToArgvW-safe; inverse of `parse_args`).
pub fn format_args(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a Windows argument string into argv (CommandLineToArgvW rules).
/// Inverse of `format_args`; do not use `split_whitespace` (drops quoting).
pub fn parse_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut n = 1usize;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    n += 1;
                }
                if chars.peek() == Some(&'"') {
                    cur.extend(std::iter::repeat('\\').take(n / 2));
                    chars.next();
                    if n % 2 == 1 {
                        cur.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                    has_token = true;
                } else {
                    cur.extend(std::iter::repeat('\\').take(n));
                    has_token = true;
                }
            }
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if !in_quotes && (c == ' ' || c == '\t') => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

/// Quote one Windows argument for CreateProcess command line (CommandLineToArgvW rules).
pub fn quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = arg.contains(' ') || arg.contains('\t') || arg.contains('"');
    if !needs_quotes {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat('\\').take(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat('\\').take(backslashes * 2));
    out.push('"');
    out
}

/// `cmd.exe /c` parses its command tail itself rather than using the C runtime
/// argument rules. Escaping the tail with `quote_arg` inserts literal
/// backslashes before quotes and breaks paths such as `C:\Program Files`.
fn create_process_command_line(program: &Path, args: &[String]) -> String {
    let mut line = quote_arg(&program.display().to_string());
    let cmd_tail = program
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
        && args.len() >= 2
        && args[..args.len() - 1]
            .iter()
            .any(|arg| arg.eq_ignore_ascii_case("/c"));
    for (index, arg) in args.iter().enumerate() {
        line.push(' ');
        if cmd_tail && index == args.len() - 1 {
            line.push_str(arg);
        } else {
            line.push_str(&quote_arg(arg));
        }
    }
    line
}

fn host_environment() -> HashMap<String, String> {
    std::env::vars().collect()
}

/// Resolve the working directory used for a root launch.
///
/// Command targets represent CLI tools. When the caller has not supplied a
/// directory, start those tools from the Windows user's profile directory so
/// that a launcher/UI process's current directory does not leak into the CLI.
/// Executable targets retain the Win32 `CreateProcess` inherited-current-
/// directory behavior, and an explicit directory always wins.
pub fn effective_working_directory(
    target: &LaunchTarget,
    requested: Option<&Path>,
) -> Result<Option<PathBuf>, LaunchError> {
    if let Some(path) = requested {
        return Ok(Some(path.to_path_buf()));
    }

    if !matches!(target, LaunchTarget::Command { .. }) {
        return Ok(None);
    }

    user_profile_directory().map(Some).ok_or_else(|| {
        LaunchError::create_process_msg("failed to resolve Windows user profile directory")
    })
}

/// Obtain the real user's profile directory without consulting the launching
/// process's current directory. On Windows this is the shell known folder,
/// which also avoids taking a potentially virtualized `USERPROFILE` value
/// from an already injected parent process.
fn user_profile_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{FOLDERID_Profile, SHGetKnownFolderPath};

        let path =
            unsafe { SHGetKnownFolderPath(&FOLDERID_Profile, Default::default(), None) }.ok()?;
        let raw_path = path.0;
        let path_len = unsafe { windows::Win32::Globalization::lstrlenW(PCWSTR(raw_path)) };
        if path_len <= 0 {
            unsafe { CoTaskMemFree(Some(raw_path.cast())) };
            return None;
        }
        let path = unsafe { std::slice::from_raw_parts(raw_path, path_len as usize) };
        let path = std::ffi::OsString::from_wide(path);
        unsafe { CoTaskMemFree(Some(raw_path.cast())) };
        Some(PathBuf::from(path))
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }
}

pub fn launch(req: LaunchRequest) -> Result<LaunchedProcess, LaunchError> {
    let host_mode = req.profile.is_none();
    if let Some(profile) = &req.profile {
        profile
            .validate()
            .map_err(|e| LaunchError::InvalidProfile(e.to_string()))?;
    }

    let working_directory =
        effective_working_directory(&req.launch, req.working_directory.as_deref())?;

    if let Some(dir) = &working_directory {
        if !dir.is_dir() {
            return Err(LaunchError::WorkingDirectoryMissing(dir.clone()));
        }
    }

    let profile_id = req.profile.as_ref().map(|p| p.id).unwrap_or_default();
    let env = if host_mode {
        // Host: real host environment, no Profile overrides, no ENVBOX_* virtualization IDs.
        host_environment()
    } else {
        build_environment_block(
            &host_environment(),
            req.profile.as_ref(),
            req.instance_id,
            profile_id,
            req.inherit_children,
            req.audit,
        )
    };

    // PATH search uses the merged environment (Profile PATH overrides Host).
    let path_env = env
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.clone());

    let (resolved, user_args) = match &req.launch {
        LaunchTarget::Executable { path } => {
            let resolved = classify_exe(path)?;
            (resolved, req.arguments.clone())
        }
        LaunchTarget::Command { command } => {
            let resolved = resolve_command(command, path_env.as_deref())?;
            (resolved, req.arguments.clone())
        }
        LaunchTarget::Packaged { aumid, .. } => {
            // Packaged roots go through Activation/Attach seams (session.rs),
            // never a raw WindowsApps exe. Keep `launch()` for Win32 golden path.
            return Err(LaunchError::create_process_msg(format!(
                "packaged target must use session start (AUMID={aumid}); \
                 do not CreateProcess a WindowsApps exe"
            )));
        }
    };

    let mut job = InstanceJob::create()?;
    let (program, mut args) = spawn_args(&resolved, &user_args, &env)?;

    // Browser Policy (ticket 52): root Chromium/Electron get the WebRTC switch.
    // Unknown engines leave the command line alone (Balanced is still fine).
    if let Some(profile) = &req.profile {
        let engine = envbox_core::BrowserEngine::from_image(&program.display().to_string());
        if engine.is_browser()
            && matches!(
                engine,
                envbox_core::BrowserEngine::Chromium
                    | envbox_core::BrowserEngine::Edge
                    | envbox_core::BrowserEngine::Electron
            )
        {
            envbox_core::ensure_chromium_webrtc_argv(&mut args, profile.browser.webrtc);
        }
    }

    let child = if host_mode {
        // Host Run: plain CreateProcess, no Runtime injection (true host view).
        spawn_plain(
            &program,
            &args,
            working_directory.as_deref(),
            &encode_environment_block(&env),
            false,
        )?
    } else {
        // Startup Fail Policy: runtime DLL must exist, match target arch, and
        // inject; never launch un-hooked (tickets 04 / 30 / 31).
        let source_runtime = crate::injection::resolve_runtime_dll_for_target(&program)?;
        let runtime_dll = crate::injection::stage_runtime_dll(&source_runtime, req.instance_id)?;
        spawn_suspended(
            &program,
            &args,
            working_directory.as_deref(),
            &encode_environment_block(&env),
            &runtime_dll,
            false,
        )?
    };

    if let Err(err) = job.assign_pid(child.pid) {
        // Startup Fail Policy: never leave a suspended Root Process behind.
        if let Err(kill_err) = child.kill_raw() {
            return Err(LaunchError::create_process_msg(format!(
                "job assign failed ({err}); also failed to terminate pid={}: {kill_err}",
                child.pid
            )));
        }
        return Err(err.into());
    }

    if !host_mode {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::ResumeThread;
            unsafe {
                if ResumeThread(child.thread.0) == u32::MAX {
                    let code = win::last_error();
                    let _ = job.terminate();
                    return Err(LaunchError::create_process(
                        code,
                        "ResumeThread failed".to_string(),
                    ));
                }
            }
        }
    }

    Ok(LaunchedProcess {
        pid: child.pid,
        instance_id: req.instance_id,
        profile_id,
        job,
        #[cfg(windows)]
        process: child.process,
        #[cfg(windows)]
        thread: child.thread,
        #[cfg(windows)]
        waited: false,
    })
}

/// Map a LaunchError from the spawn path onto ActivateError (numeric code preserved).
fn map_launch_to_activate(e: LaunchError) -> crate::activation::ActivateError {
    match e {
        LaunchError::Inject(inj) => crate::activation::ActivateError::Inject(inj),
        LaunchError::CreateProcess { code, .. } if code != 0 => {
            crate::activation::ActivateError::CreateProcess(code)
        }
        other => crate::activation::ActivateError::Resolve(other.to_string()),
    }
}

/// Spawn result used by ActivationBackend (pid + optional suspend handles).
pub struct ActivationSpawn {
    pub pid: u32,
    pub suspended: bool,
    #[cfg(windows)]
    pub process: win::SafeHandle,
    #[cfg(windows)]
    pub thread: Option<win::SafeHandle>,
}

/// Chrome and Edge can choose their persisted UI language over the OS locale.
/// Supply the Profile locale before user URLs and replace conflicting switches.
fn ensure_chromium_locale_argv(args: &mut Vec<String>, locale: &str) {
    fn remove_switch(args: &mut Vec<String>, name: &str) {
        let mut i = 0;
        while i < args.len() {
            if args[i].eq_ignore_ascii_case(name) {
                args.remove(i);
                if i < args.len() && !args[i].starts_with('-') {
                    args.remove(i);
                }
            } else if args[i]
                .split_once('=')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case(name))
            {
                args.remove(i);
            } else {
                i += 1;
            }
        }
    }

    remove_switch(args, "--lang");
    remove_switch(args, "--accept-lang");
    let language = locale.split(['-', '_']).next().unwrap_or(locale);
    let accept_languages = if language.eq_ignore_ascii_case(locale) {
        locale.to_string()
    } else {
        format!("{locale},{language}")
    };
    args.insert(0, format!("--accept-lang={accept_languages}"));
    args.insert(0, format!("--lang={locale}"));
}

fn ensure_browser_locale_argv(
    engine: envbox_core::BrowserEngine,
    args: &mut Vec<String>,
    locale: &str,
) {
    if matches!(
        engine,
        envbox_core::BrowserEngine::Chromium | envbox_core::BrowserEngine::Edge
    ) {
        ensure_chromium_locale_argv(args, locale);
    }
}

/// Spawn a Win32/Command target for the activation seam.
///
/// Profile mode: DetourCreateProcessWithDllExW (CREATE_SUSPENDED) when
/// `runtime_dll` is set; otherwise plain CreateProcess.
pub fn spawn_for_activation(
    resolved: &ResolvedCommand,
    user_args: &[String],
    req: &crate::activation::ActivationRequest,
) -> Result<ActivationSpawn, crate::activation::ActivateError> {
    let env_block = encode_environment_block(&req.environment);
    let (program, mut args) = spawn_args(resolved, user_args, &req.environment)
        .map_err(|e| crate::activation::ActivateError::Resolve(e.to_string()))?;

    // Browser Policy on the activation seam (Win32 root).
    let engine = envbox_core::BrowserEngine::from_image(&program.display().to_string());
    if let Some(locale) = &req.browser_locale {
        ensure_browser_locale_argv(engine, &mut args, locale);
    }
    if let Some(policy) = req.webrtc_policy {
        if matches!(
            engine,
            envbox_core::BrowserEngine::Chromium
                | envbox_core::BrowserEngine::Edge
                | envbox_core::BrowserEngine::Electron
        ) {
            envbox_core::ensure_chromium_webrtc_argv(&mut args, policy);
        }
    }

    if let Some(dll) = &req.runtime_dll {
        let child = spawn_suspended(
            &program,
            &args,
            req.working_directory.as_deref(),
            &env_block,
            dll,
            req.create_new_console,
        )
        .map_err(map_launch_to_activate)?;
        Ok(ActivationSpawn {
            pid: child.pid,
            suspended: true,
            #[cfg(windows)]
            process: child.process,
            #[cfg(windows)]
            thread: Some(child.thread),
        })
    } else {
        let child = spawn_plain(
            &program,
            &args,
            req.working_directory.as_deref(),
            &env_block,
            req.create_new_console,
        )
        .map_err(map_launch_to_activate)?;
        Ok(ActivationSpawn {
            pid: child.pid,
            suspended: false,
            #[cfg(windows)]
            process: child.process,
            #[cfg(windows)]
            thread: Some(child.thread),
        })
    }
}

/// Open a process handle by PID (packaged attach path).
#[cfg(windows)]
pub fn open_process_handle(pid: u32) -> Result<win::SafeHandle, crate::activation::ActivateError> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_VM_READ,
    };
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
        .map_err(|_| crate::activation::ActivateError::OpenProcess(win::last_error()))?;
        Ok(win::SafeHandle(handle))
    }
}

#[cfg(not(windows))]
pub fn open_process_handle(pid: u32) -> Result<(), crate::activation::ActivateError> {
    let _ = pid;
    Err(crate::activation::ActivateError::UnsupportedTarget(
        "open_process_handle is Windows-only".into(),
    ))
}

/// Resume a suspended activated root (PreExecution).
pub fn resume_activated(
    target: &crate::activation::ActivatedTarget,
) -> Result<(), crate::activation::ActivateError> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::ResumeThread;
        let Some(thread) = &target.thread else {
            return Ok(());
        };
        unsafe {
            if ResumeThread(thread.0) == u32::MAX {
                return Err(crate::activation::ActivateError::ResumeThread(
                    win::last_error(),
                ));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        Ok(())
    }
}

#[cfg(windows)]
struct SpawnedChild {
    pid: u32,
    process: win::SafeHandle,
    thread: win::SafeHandle,
}

#[cfg(windows)]
impl SpawnedChild {
    fn kill_raw(&self) -> Result<(), LaunchError> {
        use windows::Win32::System::Threading::TerminateProcess;
        unsafe {
            if TerminateProcess(self.process.0, 1).is_err() {
                return Err(LaunchError::create_process(
                    win::last_error(),
                    "TerminateProcess failed".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn spawn_suspended(
    program: &Path,
    args: &[String],
    working_directory: Option<&Path>,
    env_block: &[u16],
    runtime_dll: &Path,
    create_new_console: bool,
) -> Result<SpawnedChild, LaunchError> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
        STARTUPINFOW,
    };

    let dll_ansi = crate::injection::dll_path_ansi(runtime_dll)?;

    let cmdline = create_process_command_line(program, args);
    let mut cmdline_w: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();

    let mut cwd_w: Option<Vec<u16>> = working_directory.map(|d| {
        d.display()
            .to_string()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    });

    let mut si = STARTUPINFOW::default();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi = PROCESS_INFORMATION::default();

    // DetourCreateProcessWithDllExW — CREATE_SUSPENDED then Resume after Job assign.
    unsafe {
        let ok = DetourCreateProcessWithDllExW(
            PCWSTR::null(),
            PWSTR(cmdline_w.as_mut_ptr()),
            std::ptr::null(),
            std::ptr::null(),
            0,
            (CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | if create_new_console {
                    CREATE_NEW_CONSOLE
                } else {
                    Default::default()
                })
            .0,
            env_block.as_ptr() as *const _,
            cwd_w
                .as_mut()
                .map(|w| PCWSTR(w.as_ptr()))
                .unwrap_or_else(PCWSTR::null),
            &mut si,
            &mut pi,
            dll_ansi.as_ptr(),
            std::ptr::null(),
        );
        if ok == 0 {
            // Ticket 30/31: map elevation/integrity and bad-exe-format before return.
            return Err(crate::injection::map_create_process_error(win::last_error()).into());
        }
        Ok(SpawnedChild {
            pid: pi.dwProcessId,
            process: win::SafeHandle(pi.hProcess),
            thread: win::SafeHandle(pi.hThread),
        })
    }
}

#[cfg(windows)]
fn spawn_plain(
    program: &Path,
    args: &[String],
    working_directory: Option<&Path>,
    env_block: &[u16],
    create_new_console: bool,
) -> Result<SpawnedChild, LaunchError> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Threading::{
        CreateProcessW, CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
        STARTUPINFOW,
    };

    let cmdline = create_process_command_line(program, args);
    let mut cmdline_w: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();
    let mut cwd_w: Option<Vec<u16>> = working_directory.map(|d| {
        d.display()
            .to_string()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    });
    let mut si = STARTUPINFOW::default();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi = PROCESS_INFORMATION::default();
    unsafe {
        let ok = CreateProcessW(
            PCWSTR::null(),
            PWSTR(cmdline_w.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT
                | if create_new_console {
                    CREATE_NEW_CONSOLE
                } else {
                    Default::default()
                },
            Some(env_block.as_ptr() as *const _),
            cwd_w
                .as_mut()
                .map(|w| PCWSTR(w.as_ptr()))
                .unwrap_or_else(PCWSTR::null),
            &mut si,
            &mut pi,
        );
        if ok.is_err() {
            return Err(LaunchError::create_process(
                win::last_error(),
                "CreateProcessW failed".to_string(),
            ));
        }
        Ok(SpawnedChild {
            pid: pi.dwProcessId,
            process: win::SafeHandle(pi.hProcess),
            thread: win::SafeHandle(pi.hThread),
        })
    }
}

#[cfg(not(windows))]
fn spawn_plain(
    _program: &Path,
    _args: &[String],
    _working_directory: Option<&Path>,
    _env_block: &[u16],
    _create_new_console: bool,
) -> Result<SpawnedChild, LaunchError> {
    Err(LaunchError::create_process_msg(
        "CreateProcessW is Windows-only",
    ))
}

// FFI to Microsoft Detours (static lib linked in build.rs).
#[cfg(windows)]
extern "system" {
    fn DetourCreateProcessWithDllExW(
        application_name: windows::core::PCWSTR,
        command_line: windows::core::PWSTR,
        process_attributes: *const core::ffi::c_void,
        thread_attributes: *const core::ffi::c_void,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const core::ffi::c_void,
        current_directory: windows::core::PCWSTR,
        startup_info: *mut windows::Win32::System::Threading::STARTUPINFOW,
        process_information: *mut windows::Win32::System::Threading::PROCESS_INFORMATION,
        dll_name: *const u8,
        create_process_w: *const core::ffi::c_void,
    ) -> i32;
}

#[cfg(not(windows))]
fn spawn_suspended(
    program: &Path,
    args: &[String],
    working_directory: Option<&Path>,
    _env_block: &[u16],
    _runtime_dll: &Path,
    _create_new_console: bool,
) -> Result<SpawnedChild, LaunchError> {
    let _ = (program, args, working_directory);
    Err(LaunchError::create_process_msg(
        "DetourCreateProcessWithDllExW is Windows-only",
    ))
}

#[cfg(not(windows))]
struct SpawnedChild {
    pid: u32,
}

#[cfg(not(windows))]
impl SpawnedChild {
    fn kill_raw(&self) -> Result<(), LaunchError> {
        Ok(())
    }
}

fn classify_exe(path: &Path) -> Result<ResolvedCommand, LaunchError> {
    if !path.is_file() {
        return Err(CommandError::CommandNotFound(path.display().to_string()).into());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let via_comspec = ext == "cmd" || ext == "bat";
    Ok(ResolvedCommand {
        program: path.to_path_buf(),
        via_comspec,
        comspec_payload: via_comspec.then(|| path.display().to_string()),
    })
}

fn spawn_args(
    resolved: &ResolvedCommand,
    user_args: &[String],
    env: &HashMap<String, String>,
) -> Result<(PathBuf, Vec<String>), LaunchError> {
    if resolved.via_comspec {
        let comspec = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("ComSpec"))
            .map(|(_, v)| v.clone())
            .or_else(|| {
                std::env::var("ComSpec")
                    .ok()
                    .or_else(|| std::env::var("COMSPEC").ok())
            })
            .ok_or(LaunchError::ComSpecMissing)?;
        let payload = resolved
            .comspec_payload
            .clone()
            .unwrap_or_else(|| resolved.program.display().to_string());
        validate_cmd_value(&payload)?;
        for arg in user_args {
            validate_cmd_value(arg)?;
        }
        // cmd /s /c "…" — outer quotes required when payload or args have spaces.
        let mut line = String::new();
        let needs_outer = std::iter::once(payload.as_str())
            .chain(user_args.iter().map(String::as_str))
            .any(cmd_token_needs_quotes);
        if needs_outer {
            line.push('"');
        }
        line.push_str(&quote_cmd_token(&payload));
        for arg in user_args {
            line.push(' ');
            line.push_str(&quote_cmd_token(arg));
        }
        if needs_outer {
            line.push('"');
        }
        Ok((
            PathBuf::from(comspec),
            vec!["/d".into(), "/v:off".into(), "/s".into(), "/c".into(), line],
        ))
    } else {
        Ok((resolved.program.clone(), user_args.to_vec()))
    }
}

/// Quote for cmd.exe /c payload (not CreateProcess argv).
fn quote_cmd_token(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    if !cmd_token_needs_quotes(s) {
        s.to_string()
    } else {
        let trailing_slashes = s.chars().rev().take_while(|ch| *ch == '\\').count();
        format!("\"{}{}\"", s, "\\".repeat(trailing_slashes))
    }
}

fn cmd_token_needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '&' | '|' | '<' | '>' | '^' | '(' | ')'))
}

fn validate_cmd_value(value: &str) -> Result<(), LaunchError> {
    if value
        .chars()
        .any(|ch| matches!(ch, '%' | '!' | '"' | '\r' | '\n' | '\0'))
    {
        return Err(LaunchError::create_process_msg(
            "cmd.exe cannot preserve arguments containing %, !, quotes, or control characters; use an .exe target",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_locale_replaces_persisted_language_switches() {
        let mut args = vec![
            "--lang=zh-CN".into(),
            "--accept-lang".into(),
            "zh-CN,zh".into(),
            "https://example.test/".into(),
        ];
        ensure_chromium_locale_argv(&mut args, "en-US");
        assert_eq!(
            args,
            [
                "--lang=en-US",
                "--accept-lang=en-US,en",
                "https://example.test/"
            ]
        );
    }

    #[test]
    fn edge_gets_browser_locale_switches() {
        let mut args = vec!["https://example.test/".into()];
        ensure_browser_locale_argv(envbox_core::BrowserEngine::Edge, &mut args, "en-US");
        assert_eq!(
            args,
            [
                "--lang=en-US",
                "--accept-lang=en-US,en",
                "https://example.test/"
            ]
        );
    }

    #[test]
    fn cmd_command_tail_keeps_its_own_quotes() {
        let args = [
            "/d".into(),
            "/s".into(),
            "/c".into(),
            r#"""C:\Program Files\nodejs\node.exe" check.js""#.into(),
        ];
        let line = create_process_command_line(Path::new("cmd.exe"), &args);
        assert_eq!(
            line,
            r#"cmd.exe /d /s /c ""C:\Program Files\nodejs\node.exe" check.js""#
        );
    }

    #[test]
    fn quote_arg_plain() {
        assert_eq!(quote_arg("foo"), "foo");
    }

    #[test]
    fn quote_arg_spaces() {
        assert_eq!(quote_arg("a b"), "\"a b\"");
    }

    #[test]
    fn quote_arg_empty() {
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn quote_arg_embedded_quote() {
        let q = quote_arg("a\"b");
        assert!(q.starts_with('"') && q.ends_with('"'));
    }

    #[test]
    fn parse_format_args_round_trip() {
        let cases: Vec<Vec<String>> = vec![
            vec![],
            vec!["foo".into()],
            vec!["a b".into()],
            vec!["".into()],
            vec!["a\"b".into()],
            vec!["a\\".into()],
            vec!["--flag".into(), "value with space".into(), "".into()],
            vec![r"C:\path with space\app.exe".into()],
        ];
        for args in cases {
            let line = format_args(&args);
            let back = parse_args(&line);
            assert_eq!(back, args, "round-trip failed for {args:?} via {line:?}");
        }
    }

    #[test]
    fn parse_args_preserves_quoted_empty_and_spaces() {
        assert_eq!(parse_args("a \"b c\" d"), vec!["a", "b c", "d"]);
        assert_eq!(parse_args("\"\""), vec![""]);
        assert_eq!(parse_args("\"a\\\"b\""), vec!["a\"b"]);
    }

    #[test]
    fn quote_cmd_token_spaces() {
        assert_eq!(quote_cmd_token(r"C:\a b\x.cmd"), r#""C:\a b\x.cmd""#);
        assert_eq!(
            quote_cmd_token("C:\\dir with space\\"),
            "\"C:\\dir with space\\\\\""
        );
        assert_eq!(quote_cmd_token("safe&ver"), "\"safe&ver\"");
    }

    #[test]
    fn comspec_payload_quotes_path_with_spaces() {
        let resolved = ResolvedCommand {
            program: PathBuf::from(r"C:\Program Files\app\run.cmd"),
            via_comspec: true,
            comspec_payload: Some(r"C:\Program Files\app\run.cmd".into()),
        };
        let env = HashMap::from([("ComSpec".into(), r"C:\Windows\System32\cmd.exe".into())]);
        let (prog, args) = spawn_args(&resolved, &["a b".into()], &env).unwrap();
        assert!(prog.ends_with("cmd.exe"));
        assert_eq!(args[0], "/d");
        assert_eq!(args[3], "/c");
        let line = &args[4];
        assert!(line.starts_with('"') && line.ends_with('"'));
        assert!(line.contains(r#"C:\Program Files\app\run.cmd"#));
        assert!(line.contains("\"a b\""));
    }

    #[test]
    fn comspec_rejects_expanding_arguments() {
        let resolved = ResolvedCommand {
            program: PathBuf::from(r"C:\app\run.cmd"),
            via_comspec: true,
            comspec_payload: None,
        };
        let env = HashMap::from([("ComSpec".into(), r"C:\Windows\System32\cmd.exe".into())]);
        assert!(spawn_args(&resolved, &["%PATH%".into()], &env).is_err());
        assert!(spawn_args(&resolved, &["a!b".into()], &env).is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "starts a real cmd.exe process"]
    fn comspec_metacharacters_reach_batch_as_one_argument() {
        let base = std::env::temp_dir().join(format!("envbox-batch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let script = base.join("read-arg.cmd");
        let output = base.join("arg.txt");
        std::fs::write(
            &script,
            format!("@echo off\r\n>\"{}\" echo %1\r\n", output.display()),
        )
        .unwrap();
        let resolved = classify_exe(&script).unwrap();
        let environment = host_environment();
        let block = encode_environment_block(&environment);
        for argument in ["safe&ver", "safe|ver", "safe>ver", "safe^ver", "safe(ver)"] {
            let (program, args) = spawn_args(&resolved, &[argument.into()], &environment).unwrap();
            let child = spawn_plain(&program, &args, None, &block, false).unwrap();
            let wait = unsafe {
                windows::Win32::System::Threading::WaitForSingleObject(child.process.0, 5_000)
            };
            assert_eq!(wait.0, 0, "batch did not exit for {argument:?}");
            assert_eq!(
                std::fs::read_to_string(&output).unwrap().trim(),
                format!("\"{argument}\""),
                "batch changed argument {argument:?}"
            );
            std::fs::remove_file(&output).unwrap();
        }
        let _ = std::fs::remove_file(&script);
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_dir(&base);
    }

    #[test]
    fn explicit_working_directory_wins_for_commands() {
        let explicit = Path::new(r"C:\work\project");
        let selected = effective_working_directory(
            &LaunchTarget::Command {
                command: "my-cli".into(),
            },
            Some(explicit),
        )
        .unwrap();
        assert_eq!(selected.as_deref(), Some(explicit));
    }

    #[test]
    fn executable_without_working_directory_keeps_inherited_semantics() {
        let selected = effective_working_directory(
            &LaunchTarget::Executable {
                path: PathBuf::from(r"C:\Program Files\my-gui.exe"),
            },
            None,
        )
        .unwrap();
        assert_eq!(selected, None);
    }

    #[test]
    fn command_without_working_directory_uses_user_profile_not_launcher_directory() {
        let selected = effective_working_directory(
            &LaunchTarget::Command {
                command: "my-cli".into(),
            },
            None,
        )
        .unwrap()
        .expect("user profile directory should be available");
        let launcher_directory = std::env::current_exe()
            .expect("test executable path")
            .parent()
            .expect("test executable directory")
            .to_path_buf();
        assert_ne!(selected, launcher_directory);

        #[cfg(windows)]
        assert_eq!(
            selected,
            user_profile_directory().expect("known user profile directory")
        );

        #[cfg(not(windows))]
        assert_eq!(
            selected,
            PathBuf::from(std::env::var_os("HOME").expect("HOME"))
        );
    }
}
