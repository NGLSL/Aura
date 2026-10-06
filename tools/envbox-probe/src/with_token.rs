//! Native WithToken boundary probe. Any unexpected child stays suspended and
//! is terminated before it can execute application code.

use std::os::windows::ffi::OsStrExt;
use std::process::ExitCode;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED, PROCESS_INFORMATION, STARTUPINFOW,
};

#[link(name = "advapi32")]
unsafe extern "system" {
    fn CreateProcessWithTokenW(
        token: HANDLE,
        logon_flags: u32,
        app: PCWSTR,
        command: PWSTR,
        flags: u32,
        environment: *const std::ffi::c_void,
        directory: PCWSTR,
        startup: *const STARTUPINFOW,
        process: *mut PROCESS_INFORMATION,
    ) -> i32;
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

pub fn run() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("WithToken current_exe: {error}");
            return ExitCode::FAILURE;
        }
    };
    let application: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut command: Vec<u16> = format!("\"{}\" --child", exe.display())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();
    let succeeded = unsafe {
        CreateProcessWithTokenW(
            HANDLE::default(), // Invalid token: Host must report a native error.
            0,
            PCWSTR(application.as_ptr()),
            PWSTR(command.as_mut_ptr()),
            CREATE_SUSPENDED.0,
            std::ptr::null(),
            PCWSTR::null(),
            &startup,
            &mut process_info,
        ) != 0
    };
    // Capture immediately, before output, cleanup, or another Windows call.
    let error = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    let has_child = process_info.dwProcessId != 0
        || !process_info.hProcess.is_invalid()
        || !process_info.hThread.is_invalid();
    let process = OwnedHandle(process_info.hProcess);
    let _thread = OwnedHandle(process_info.hThread);
    if !process.0.is_invalid() {
        unsafe {
            if let Err(error) = TerminateProcess(process.0, 1) {
                eprintln!("WithToken unexpected-child cleanup: {error}");
                return ExitCode::FAILURE;
            }
            let wait = WaitForSingleObject(process.0, 5_000);
            if wait != WAIT_OBJECT_0 {
                if wait == WAIT_FAILED {
                    let cleanup_error = std::io::Error::last_os_error();
                    eprintln!("WithToken child cleanup wait failed: {cleanup_error}");
                } else {
                    eprintln!("WithToken child cleanup did not complete: wait={}", wait.0);
                }
                return ExitCode::FAILURE;
            }
        }
    }
    let runtime = std::env::var("ENVBOX_RUNTIME_LOADED").as_deref() == Ok("1");
    let controlled = runtime && std::env::var("ENVBOX_STARTUP_GATE").as_deref() == Ok("1");
    println!("=== CREATEPROCESSWITHTOKENW ===");
    println!("RuntimeLoaded: {runtime}");
    println!("Controlled: {controlled}");
    println!("Token: invalid-null");
    println!("Succeeded: {succeeded}");
    println!("WindowsError: {error}");
    println!("ChildCreated: {has_child}");
    if !succeeded
        && !has_child
        && if controlled {
            error == 50
        } else {
            error != 50 && error != 0
        }
    {
        ExitCode::SUCCESS
    } else {
        eprintln!("WithToken boundary did not match the expected native/controlled result");
        ExitCode::FAILURE
    }
}
