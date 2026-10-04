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
    #[error("OpenJobObject failed (GetLastError={0})")]
    Open(u32),
    #[error("invalid named Job Object name")]
    InvalidName,
    #[error("tracking Job has unexpected limit flags: {0:#x}")]
    UnexpectedLimits(u32),
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobStats {
    pub active_processes: u32,
    pub total_user_time_100ns: u64,
    pub total_page_fault_count: u32,
    /// Current process IDs in the Job. The list order is unspecified; callers
    /// that need the root PID must use the session handoff marker.
    pub process_ids: Vec<u32>,
}

/// RAII Job Object used for tracking and explicit Stop.
/// Dropping the handle leaves the instance running so GUI exit and installer
/// upgrades do not terminate user applications.
pub struct InstanceJob {
    #[cfg(windows)]
    handle: win::SafeHandle,
    pub assigned_pids: Vec<u32>,
}

/// A creation-time Job attribute capability, constructible only by its owner.
/// The borrowed lifetime prevents closing the Job during CreateProcess.
#[derive(Debug, Clone, Copy)]
pub struct JobAssignment<'a> {
    #[cfg(windows)]
    pub(crate) handle: windows::Win32::Foundation::HANDLE,
    _owner: std::marker::PhantomData<&'a InstanceJob>,
}

impl InstanceJob {
    /// Recovery accepts only the unmodified tracking Job policy used by this launcher.
    pub fn verify_tracking_limits(&self) -> Result<(), JobError> {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::JobObjects::{
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            QueryInformationJobObject(
                self.handle.0,
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
                None,
            )
            .map_err(|_| JobError::Query(last_error()))?;
            if limits.BasicLimitInformation.LimitFlags.0 != 0 {
                return Err(JobError::UnexpectedLimits(
                    limits.BasicLimitInformation.LimitFlags.0,
                ));
            }
        }
        Ok(())
    }
    pub fn creation_assignment(&self) -> JobAssignment<'_> {
        JobAssignment {
            #[cfg(windows)]
            handle: self.handle.0,
            _owner: std::marker::PhantomData,
        }
    }
    pub fn create() -> Result<Self, JobError> {
        Self::create_with_name(None)
    }

    /// Create a Job with a local-session name. Named Jobs are used by the
    /// Windows Terminal bridge: the GUI creates and owns the object, while
    /// the short-lived `terminal-run` process opens the same object before it
    /// starts the actual CLI.
    pub fn create_named(name: &str) -> Result<Self, JobError> {
        Self::create_with_name(Some(name))
    }

    /// Create a new ownership object. A name collision must never authorize
    /// cleanup or termination of another Run's Job.
    pub fn create_named_exclusive(name: &str) -> Result<Self, JobError> {
        #[cfg(windows)]
        {
            use windows::core::PCWSTR;
            use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
            use windows::Win32::System::JobObjects::CreateJobObjectW;
            let name = wide_name(name)?;
            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR(name.as_ptr()))
                    .map_err(|_| JobError::Create(last_error()))?;
                let error = GetLastError();
                let owned = win::SafeHandle(handle);
                if error == ERROR_ALREADY_EXISTS {
                    return Err(JobError::Create(error.0));
                }
                Ok(Self {
                    handle: owned,
                    assigned_pids: Vec::new(),
                })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err(JobError::Create(0))
        }
    }

    /// Open a Job created by another process in this Windows session.
    pub fn open_named(name: &str) -> Result<Self, JobError> {
        #[cfg(windows)]
        {
            use windows::core::PCWSTR;
            use windows::Win32::System::JobObjects::OpenJobObjectW;

            let name = wide_name(name)?;
            // Only request the operations EnvBox needs. The name is generated
            // internally and is never built from user command text.
            const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x0001;
            const JOB_OBJECT_QUERY: u32 = 0x0004;
            const JOB_OBJECT_TERMINATE: u32 = 0x0008;
            let access = JOB_OBJECT_ASSIGN_PROCESS | JOB_OBJECT_QUERY | JOB_OBJECT_TERMINATE;

            unsafe {
                let handle = OpenJobObjectW(access, false, PCWSTR(name.as_ptr()))
                    .map_err(|_| JobError::Open(last_error()))?;
                Ok(Self {
                    handle: win::SafeHandle(handle),
                    assigned_pids: Vec::new(),
                })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err(JobError::Open(0))
        }
    }

    fn create_with_name(name: Option<&str>) -> Result<Self, JobError> {
        #[cfg(windows)]
        {
            use windows::core::PCWSTR;
            use windows::Win32::System::JobObjects::CreateJobObjectW;

            let name = match name {
                Some(name) => wide_name(name)?,
                None => vec![0],
            };
            unsafe {
                let name_ptr = if name.len() == 1 {
                    PCWSTR::null()
                } else {
                    PCWSTR(name.as_ptr())
                };
                let handle =
                    CreateJobObjectW(None, name_ptr).map_err(|_| JobError::Create(last_error()))?;

                Ok(Self {
                    handle: win::SafeHandle(handle),
                    assigned_pids: Vec::new(),
                })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = name;
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
                    process_ids: query_process_ids(self.handle.0)?,
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

#[cfg(windows)]
fn wide_name(name: &str) -> Result<Vec<u16>, JobError> {
    if name.is_empty() || name.contains('\0') {
        return Err(JobError::InvalidName);
    }
    Ok(name.encode_utf16().chain(std::iter::once(0)).collect())
}

#[cfg(windows)]
unsafe fn query_process_ids(
    handle: windows::Win32::Foundation::HANDLE,
) -> Result<Vec<u32>, JobError> {
    use windows::Win32::System::JobObjects::{
        JobObjectBasicProcessIdList, QueryInformationJobObject,
    };

    // The API reports the required size through a failed query only on some
    // Windows builds. Start with a small buffer and grow on the documented
    // ERROR_INSUFFICIENT_BUFFER path, bounded so a corrupt target cannot make
    // the GUI allocate unbounded memory.
    const HEADER: usize = std::mem::size_of::<u32>() * 2;
    const MAX_IDS: usize = 16_384;
    let mut capacity = 32usize;
    loop {
        let bytes = HEADER + capacity * std::mem::size_of::<usize>();
        let mut buffer = vec![0u8; bytes];
        let result = QueryInformationJobObject(
            handle,
            JobObjectBasicProcessIdList,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            None,
        );
        if result.is_ok() {
            let assigned = std::ptr::read_unaligned(buffer.as_ptr().cast::<u32>());
            let count = std::ptr::read_unaligned(buffer.as_ptr().add(4).cast::<u32>()) as usize;
            let count = count.min(capacity).min(assigned as usize).min(MAX_IDS);
            let ids = buffer.as_ptr().add(HEADER).cast::<usize>();
            return Ok((0..count)
                .map(|index| std::ptr::read_unaligned(ids.add(index)) as u32)
                .collect());
        }
        let error = last_error();
        // Windows versions have reported both ERROR_INSUFFICIENT_BUFFER
        // (122) and ERROR_MORE_DATA (234) for this information class.
        if !matches!(error, 122 | 234) || capacity >= MAX_IDS {
            return Err(JobError::Query(error));
        }
        capacity = (capacity * 2).min(MAX_IDS);
    }
}

impl Drop for InstanceJob {
    fn drop(&mut self) {
        // SafeHandle closes only the tracking handle. Applications stay alive.
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[cfg(windows)]
    #[test]
    fn exclusive_job_collision_leaves_existing_job_owned() {
        let name = format!("Local\\AuraExclusiveJobTest-{}", uuid::Uuid::new_v4());
        let original = super::InstanceJob::create_named_exclusive(&name).unwrap();
        assert!(matches!(
            super::InstanceJob::create_named_exclusive(&name),
            Err(super::JobError::Create(183))
        ));
        assert_eq!(original.stats().unwrap().active_processes, 0);
        let reopened = super::InstanceJob::open_named(&name).unwrap();
        assert_eq!(reopened.stats().unwrap().active_processes, 0);
    }
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

    #[test]
    fn named_job_can_be_opened_by_handoff_process() {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let name = format!("Local\\Aura-test-{}", uuid::Uuid::new_v4());
        let mut owner = InstanceJob::create_named(&name).expect("create named job");
        let mut handoff = InstanceJob::open_named(&name).expect("open named job");
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/d", "/s", "/c", "ping -n 30 127.0.0.1 >nul"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn child");
        handoff.assign_pid(child.id()).expect("assign child");
        let stats = owner.stats().expect("query named job");
        assert!(stats.process_ids.contains(&child.id()));
        owner.close().expect("stop named job");
        let _ = child.wait();
    }
}
