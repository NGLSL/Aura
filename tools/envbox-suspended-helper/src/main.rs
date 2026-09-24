//! envbox-suspended-helper: ticket 35 seam.
//!
//! Creates a child with `CREATE_SUSPENDED` (the same call shape an installer or
//! debugger uses), reports the primary-thread suspend count, optionally holds
//! without Resume, then ResumeThread and waits.
//!
//! Usage (under `envbox run`):
//!   envbox-suspended-helper [--hold] -- <child_exe> [args...]
//!
//! Output lines:
//!   SUSPEND_COUNT=<n>
//!   STILL_SUSPENDED
//!   RESUMED_OK
//!   HELD_NO_RESUME
//!   CHILD_EXIT=<code>

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hold = args.iter().any(|a| a == "--hold");
    let dash = args.iter().position(|a| a == "--").unwrap_or(usize::MAX);
    let child_args: Vec<String> = if dash < args.len() {
        args[dash + 1..].to_vec()
    } else {
        args.iter()
            .filter(|a| a.as_str() != "--hold" && a.as_str() != "--")
            .cloned()
            .collect()
    };
    if child_args.is_empty() {
        eprintln!("usage: envbox-suspended-helper [--hold] -- <child_exe> [args...]");
        return ExitCode::FAILURE;
    }
    match run_suspended(&child_args, hold) {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_suspended(child_args: &[String], hold: bool) -> Result<u32, String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let program = &child_args[0];
    let mut cmdline = format!("\"{program}\"");
    for a in &child_args[1..] {
        cmdline.push(' ');
        if a.is_empty() || a.contains(' ') || a.contains('\t') || a.contains('"') {
            cmdline.push('"');
            cmdline.push_str(&a.replace('"', "\"\""));
            cmdline.push('"');
        } else {
            cmdline.push_str(a);
        }
    }
    let mut cmdline_w: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();

    let mut si = STARTUPINFOW::default();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi = PROCESS_INFORMATION::default();

    let created = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            PWSTR(cmdline_w.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &mut si,
            &mut pi,
        )
    };
    if created.is_err() {
        return Err(format!("CreateProcessW failed ({})", last_error()));
    }

    // Safety: pi handles are owned here; closed on every exit path.
    let process = pi.hProcess;
    let thread = pi.hThread;
    let _guard = HandleGuard([process, thread]);

    // Give the injected Runtime a moment to finish detach while still suspended.
    unsafe {
        let _ = WaitForSingleObject(process, 50);
    }

    let count = thread_suspend_count(thread).unwrap_or(0);
    println!("SUSPEND_COUNT={count}");
    if count > 0 {
        println!("STILL_SUSPENDED");
    } else {
        println!("NOT_SUSPENDED");
    }

    if hold {
        // Ticket 35 negative path: never Resume — child must not make progress.
        let terminated = unsafe { TerminateProcess(process, 0) };
        if terminated.is_err() {
            return Err(format!(
                "TerminateProcess failed ({}) after HOLD_NO_RESUME",
                last_error()
            ));
        }
        println!("HELD_NO_RESUME");
        return Ok(0);
    }

    let prev = unsafe { ResumeThread(thread) };
    if prev == u32::MAX {
        return Err(format!("ResumeThread failed ({})", last_error()));
    }
    println!("RESUME_THREAD_PREV={prev}");

    let wait = unsafe { WaitForSingleObject(process, INFINITE) };
    if wait != WAIT_OBJECT_0 {
        return Err(format!("WaitForSingleObject failed (wait={:?})", wait.0));
    }
    let mut code = 0u32;
    unsafe {
        let _ = GetExitCodeProcess(process, &mut code);
    }
    println!("RESUMED_OK");
    println!("CHILD_EXIT={code}");
    Ok(code)
}

struct HandleGuard([windows::Win32::Foundation::HANDLE; 2]);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        for h in self.0 {
            if !h.is_invalid() {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(h);
                }
            }
        }
    }
}

fn last_error() -> u32 {
    use windows::Win32::Foundation::GetLastError;
    unsafe { GetLastError().0 }
}

/// Primary-thread suspend count via `NtQueryInformationThread(ThreadSuspendCount)`.
fn thread_suspend_count(
    thread: windows::Win32::Foundation::HANDLE,
) -> Option<u32> {
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::PCWSTR;

    // ThreadSuspendCount = 35 (0x23).
    const THREAD_SUSPEND_COUNT: i32 = 35;

    type NtQueryInformationThread = unsafe extern "system" fn(
        thread: isize,
        info_class: i32,
        info: *mut core::ffi::c_void,
        info_len: u32,
        ret_len: *mut u32,
    ) -> i32;

    unsafe {
        let name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
        let module = LoadLibraryW(PCWSTR(name.as_ptr())).ok()?;
        let Some(sym) = GetProcAddress(module, windows::core::s!("NtQueryInformationThread"))
        else {
            let _ = FreeLibrary(module);
            return None;
        };
        let f: NtQueryInformationThread = std::mem::transmute(sym);
        let mut count = 0u32;
        let mut ret_len = 0u32;
        let status = f(
            thread.0 as isize,
            THREAD_SUSPEND_COUNT,
            &mut count as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
            &mut ret_len,
        );
        let _ = FreeLibrary(module);
        if status == 0 {
            Some(count)
        } else {
            None
        }
    }
}
