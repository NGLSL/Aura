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
            use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
            use std::os::windows::process::ExitStatusExt;

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

    /// Stop the Process Tree Instance by closing the Job handle
    /// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), not TerminateJobObject.
    pub fn stop(&mut self) -> Result<(), crate::job::JobError> {
        self.job.close()
    }

    pub fn job_stats(&self) -> Result<crate::job::JobStats, LaunchError> {
        Ok(self.job.stats()?)
    }
}

/// Join argv for display/edit (CommandLineToArgvW-safe; inverse of `parse_args`).
pub fn format_args(args: &[String]) -> String {
    args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ")
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

fn host_environment() -> HashMap<String, String> {
    std::env::vars().collect()
}

pub fn launch(req: LaunchRequest) -> Result<LaunchedProcess, LaunchError> {
    let host_mode = req.profile.is_none();
    if let Some(profile) = &req.profile {
        profile
            .validate()
            .map_err(|e| LaunchError::InvalidProfile(e.to_string()))?;
    }

    if let Some(dir) = &req.working_directory {
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
    let path_env = env.get("PATH").cloned();

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
            req.working_directory.as_deref(),
            &encode_environment_block(&env),
        )?
    } else {
        // Startup Fail Policy: runtime DLL must exist, match target arch, and
        // inject; never launch un-hooked (tickets 04 / 30 / 31).
        let runtime_dll = crate::injection::resolve_runtime_dll_for_target(&program)?;
        spawn_suspended(
            &program,
            &args,
            req.working_directory.as_deref(),
            &encode_environment_block(&env),
            &runtime_dll,
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
    let (program, mut args) = spawn_args(resolved, user_args, &req.environment).map_err(|e| {
        crate::activation::ActivateError::Resolve(e.to_string())
    })?;

    // Browser Policy on the activation seam (Win32 root).
    if let Some(policy) = req.webrtc_policy {
        let engine = envbox_core::BrowserEngine::from_image(&program.display().to_string());
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
pub fn open_process_handle(
    pid: u32,
) -> Result<win::SafeHandle, crate::activation::ActivateError> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_VM_READ,
    };
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
        .map_err(|_| {
            crate::activation::ActivateError::OpenProcess(win::last_error())
        })?;
        Ok(win::SafeHandle(handle))
    }
}

#[cfg(not(windows))]
pub fn open_process_handle(
    pid: u32,
) -> Result<(), crate::activation::ActivateError> {
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
) -> Result<SpawnedChild, LaunchError> {
    use windows::Win32::System::Threading::{
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};

    let dll_ansi = crate::injection::dll_path_ansi(runtime_dll)?;

    let mut cmdline = quote_arg(&program.display().to_string());
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(&quote_arg(a));
    }
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
            (CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT).0,
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
) -> Result<SpawnedChild, LaunchError> {
    use windows::Win32::System::Threading::{
        CreateProcessW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};

    let mut cmdline = quote_arg(&program.display().to_string());
    for a in args {
        cmdline.push(' ');
        cmdline.push_str(&quote_arg(a));
    }
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
            CREATE_UNICODE_ENVIRONMENT,
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
            .or_else(|| std::env::var("ComSpec").ok().or_else(|| std::env::var("COMSPEC").ok()))
            .ok_or(LaunchError::ComSpecMissing)?;
        let payload = resolved
            .comspec_payload
            .clone()
            .unwrap_or_else(|| resolved.program.display().to_string());
        // cmd /s /c "…" — outer quotes required when payload or args have spaces.
        let mut line = String::new();
        let needs_outer = payload.contains(' ')
            || payload.contains('\t')
            || payload.contains('"')
            || user_args.iter().any(|a| {
                a.contains(' ') || a.contains('\t') || a.contains('"')
            });
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
        Ok((PathBuf::from(comspec), vec!["/d".into(), "/s".into(), "/c".into(), line]))
    } else {
        Ok((resolved.program.clone(), user_args.to_vec()))
    }
}

/// Quote for cmd.exe /c payload (not CreateProcess argv).
fn quote_cmd_token(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".into();
    }
    if s.contains(' ') || s.contains('\t') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(args[2], "/c");
        let line = &args[3];
        assert!(line.starts_with('"') && line.ends_with('"'));
        assert!(line.contains(r#"C:\Program Files\app\run.cmd"#));
        assert!(line.contains("\"a b\""));
    }
}
