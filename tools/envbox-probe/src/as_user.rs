//! Exercise the token-based child creation path without involving another app.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TokenPrimary, TOKEN_ALL_ACCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken,
    TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW,
};

pub enum Outcome {
    Succeeded(String),
    SkippedPrivilege(&'static str, i32),
}

fn permission_skip(stage: &'static str) -> Result<Outcome, String> {
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code @ (5 | 1314)) => Ok(Outcome::SkippedPrivilege(stage, code)),
        _ => Err(format!("{stage}: {error}")),
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct OutputFile(PathBuf);

impl Drop for OutputFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

pub fn spawn_child() -> Result<Outcome, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let output_file = OutputFile(std::env::temp_dir().join(format!(
        "envbox-probe-as-user-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("clock: {e}"))?
            .as_nanos()
    )));
    let exe_wide = wide(exe.as_os_str());
    let mut command_wide = wide(std::ffi::OsStr::new(&format!(
        "\"{}\" --as-user-child-output \"{}\"",
        exe.display(),
        output_file.0.display()
    )));

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token).is_err() {
            return permission_skip("OpenProcessToken");
        }
        let token = OwnedHandle(token);
        let mut primary = HANDLE::default();
        if DuplicateTokenEx(
            token.0,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
        .is_err()
        {
            return permission_skip("DuplicateTokenEx");
        }
        let primary = OwnedHandle(primary);

        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut process_info = PROCESS_INFORMATION::default();
        if CreateProcessAsUserW(
            primary.0,
            PCWSTR(exe_wide.as_ptr()),
            PWSTR(command_wide.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_NO_WINDOW,
            None,
            PCWSTR::null(),
            &startup,
            &mut process_info,
        )
        .is_err()
        {
            return permission_skip("CreateProcessAsUserW");
        }
        let process = OwnedHandle(process_info.hProcess);
        let _thread = OwnedHandle(process_info.hThread);
        let wait = WaitForSingleObject(process.0, 30_000);
        if wait == WAIT_TIMEOUT {
            let _ = TerminateProcess(process.0, 1);
            return Err("as-user child timed out after 30 seconds".into());
        }
        if wait != WAIT_OBJECT_0 {
            return Err(format!("WaitForSingleObject returned {}", wait.0));
        }
        let mut exit_code = 0;
        GetExitCodeProcess(process.0, &mut exit_code)
            .map_err(|e| format!("GetExitCodeProcess: {e}"))?;
        if exit_code != 0 {
            return Err(format!("as-user child exited with code {exit_code}"));
        }
    }

    let output = std::fs::read_to_string(&output_file.0)
        .map_err(|e| format!("read as-user child output: {e}"))?;
    Ok(Outcome::Succeeded(output))
}
