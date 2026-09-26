//! Job Object for RuntimeInstance lifecycle (not a security boundary).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum JobError {
    #[error("CreateJobObject failed (GetLastError={0})")]
    Create(u32),
    #[error("AssignProcessToJobObject failed (GetLastError={0})")]
    Assign(u32),
    #[error("TerminateJobObject failed (GetLastError={0})")]
    Terminate(u32),
    #[error("QueryInformationJobObject failed (GetLastError={0})")]
    Query(u32),
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};

    /// Capture GetLastError immediately after a failed Win32 call.
    pub fn last_error() -> u32 {
        unsafe { GetLastError().0 }
    }

    /// RAII wrapper for a process/thread/job HANDLE.
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

#[cfg(windows)]
use win::last_error;

/// Active process count and total user time from the job (when available).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobStats {
    pub active_processes: u32,
    pub total_user_time_100ns: u64,
    pub total_page_fault_count: u32,
}

/// RAII Job Object used for tracking and explicit Stop.
/// Dropping the handle leaves the instance running so GUI exit and installer
/// upgrades do not terminate user applications.
pub struct InstanceJob {
    #[cfg(windows)]
    handle: win::SafeHandle,
    pub assigned_pids: Vec<u32>,
}

impl InstanceJob {
    pub fn create() -> Result<Self, JobError> {
        #[cfg(windows)]
        {
            use windows::Win32::System::JobObjects::CreateJobObjectW;
            use windows::core::PCWSTR;

            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR::null())
                    .map_err(|_| JobError::Create(last_error()))?;

                Ok(Self {
                    handle: win::SafeHandle(handle),
                    assigned_pids: Vec::new(),
                })
            }
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                assigned_pids: Vec::new(),
            })
        }
    }

    pub fn assign_pid(&mut self, pid: u32) -> Result<(), JobError> {
        #[cfg(windows)]
        {
            use windows::Win32::System::JobObjects::AssignProcessToJobObject;
            use windows::Win32::System::Threading::{
                OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
            };

            unsafe {
                let raw = match OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) {
                    Ok(h) => h,
                    Err(_) => return Err(JobError::Assign(last_error())),
                };
                let process = win::SafeHandle(raw);
                if AssignProcessToJobObject(self.handle.0, process.0).is_err() {
                    return Err(JobError::Assign(last_error()));
                }
            }
        }
        self.assigned_pids.push(pid);
        Ok(())
    }

    pub fn stats(&self) -> Result<JobStats, JobError> {
        #[cfg(windows)]
        {
            use windows::Win32::System::JobObjects::{
                JobObjectBasicAccountingInformation, QueryInformationJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            };
            unsafe {
                let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                if QueryInformationJobObject(
                    self.handle.0,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    std::mem::size_of_val(&info) as u32,
                    None,
                )
                .is_err()
                {
                    return Err(JobError::Query(last_error()));
                }
                Ok(JobStats {
                    active_processes: info.ActiveProcesses,
                    total_user_time_100ns: info.TotalUserTime as u64,
                    total_page_fault_count: info.TotalPageFaultCount,
                })
            }
        }
        #[cfg(not(windows))]
        {
            Ok(JobStats::default())
        }
    }

    pub fn terminate(&self) -> Result<(), JobError> {
        #[cfg(windows)]
        {
            use windows::Win32::System::JobObjects::TerminateJobObject;
            unsafe {
                if TerminateJobObject(self.handle.0, 1).is_err() {
                    return Err(JobError::Terminate(last_error()));
                }
            }
        }
        Ok(())
    }

    /// Explicitly stop the Process Tree Instance, then release the Job handle.
    /// Ordinary Drop only releases tracking and intentionally does not stop it.
    pub fn close(&mut self) -> Result<(), JobError> {
        self.terminate()?;
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HANDLE;
            let handle = std::mem::replace(&mut self.handle, win::SafeHandle(HANDLE::default()));
            drop(handle);
        }
        Ok(())
    }
}

impl Drop for InstanceJob {
    fn drop(&mut self) {
        // SafeHandle closes only the tracking handle. Applications stay alive.
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;

    #[test]
    fn dropping_job_keeps_assigned_process_alive() {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/d", "/s", "/c", "ping -n 30 127.0.0.1 >nul"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn child");
        let mut job = InstanceJob::create().expect("create job");
        job.assign_pid(child.id()).expect("assign child");

        drop(job);
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert!(child.try_wait().expect("query child").is_none());
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn explicit_close_stops_assigned_process() {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/d", "/s", "/c", "ping -n 30 127.0.0.1 >nul"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn child");
        let mut job = InstanceJob::create().expect("create job");
        job.assign_pid(child.id()).expect("assign child");

        job.close().expect("explicit stop");
        let status = child.wait().expect("wait stopped child");

        assert!(!status.success());
    }
}
