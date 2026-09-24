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
    #[error("failed to create process: {0}")]
    CreateProcess(String),
    #[error("comspec not set")]
    ComSpecMissing,
    #[error(transparent)]
    Inject(#[from] crate::injection::InjectError),
}

pub struct LaunchRequest {
    pub launch: LaunchTarget,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub profile: EnvironmentProfile,
    pub instance_id: Uuid,
    /// Child processes inherit the Environment Profile (Application.inherit_children).
    pub inherit_children: bool,
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
mod win {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};

    pub fn last_error() -> u32 {
        unsafe { GetLastError().0 }
    }

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

    pub fn stop(&mut self) -> Result<(), LaunchError> {
        self.job.terminate()?;
        Ok(())
    }

    pub fn job_stats(&self) -> Result<crate::job::JobStats, LaunchError> {
        Ok(self.job.stats()?)
    }
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
    req.profile
        .validate()
        .map_err(|e| LaunchError::InvalidProfile(e.to_string()))?;

    if let Some(dir) = &req.working_directory {
        if !dir.is_dir() {
            return Err(LaunchError::WorkingDirectoryMissing(dir.clone()));
        }
    }

    let env = build_environment_block(
        &host_environment(),
        Some(&req.profile),
        req.instance_id,
        req.profile.id,
        req.inherit_children,
    );

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
    };

    let mut job = InstanceJob::create()?;
    let (program, args) = spawn_args(&resolved, &user_args, &env)?;

    // Startup Fail Policy: runtime DLL must exist and inject; never launch un-hooked.
    let runtime_dll = crate::injection::resolve_runtime_dll()?;

    let child = spawn_suspended(
        &program,
        &args,
        req.working_directory.as_deref(),
        &encode_environment_block(&env),
        &runtime_dll,
    )?;

    if let Err(err) = job.assign_pid(child.pid) {
        // Startup Fail Policy: never leave a suspended Root Process behind.
        if let Err(kill_err) = child.kill_raw() {
            return Err(LaunchError::CreateProcess(format!(
                "job assign failed ({err}); also failed to terminate pid={}: {kill_err}",
                child.pid
            )));
        }
        return Err(err.into());
    }

    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::ResumeThread;
        unsafe {
            if ResumeThread(child.thread.0) == u32::MAX {
                let code = win::last_error();
                let _ = job.terminate();
                return Err(LaunchError::CreateProcess(format!(
                    "ResumeThread failed (GetLastError={code})"
                )));
            }
        }
    }

    Ok(LaunchedProcess {
        pid: child.pid,
        instance_id: req.instance_id,
        profile_id: req.profile.id,
        job,
        #[cfg(windows)]
        process: child.process,
        #[cfg(windows)]
        thread: child.thread,
        #[cfg(windows)]
        waited: false,
    })
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
                return Err(LaunchError::CreateProcess(format!(
                    "TerminateProcess failed (GetLastError={})",
                    win::last_error()
                )));
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
            return Err(crate::injection::InjectError::DetourCreateProcess(
                win::last_error(),
            )
            .into());
        }
        Ok(SpawnedChild {
            pid: pi.dwProcessId,
            process: win::SafeHandle(pi.hProcess),
            thread: win::SafeHandle(pi.hThread),
        })
    }
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
    Err(LaunchError::CreateProcess(
        "DetourCreateProcessWithDllExW is Windows-only".into(),
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
