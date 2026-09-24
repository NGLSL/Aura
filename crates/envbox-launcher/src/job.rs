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
}

#[cfg(windows)]
fn last_error() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

#[cfg(windows)]
struct ProcessHandle(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// RAII Job Object. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is set at creation.
pub struct InstanceJob {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    pub assigned_pids: Vec<u32>,
}

impl InstanceJob {
    pub fn create() -> Result<Self, JobError> {
        #[cfg(windows)]
        {
            use windows::Win32::System::JobObjects::{
                CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };
            use windows::core::PCWSTR;

            unsafe {
                let handle =
                    CreateJobObjectW(None, PCWSTR::null()).map_err(|_| JobError::Create(last_error()))?;

                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32,
                )
                .is_err()
                {
                    let code = last_error();
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                    return Err(JobError::Create(code));
                }

                Ok(Self {
                    handle,
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
                let raw = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)
                    .map_err(|_| JobError::Assign(last_error()))?;
                let process = ProcessHandle(raw);
                AssignProcessToJobObject(self.handle, process.0)
                    .map_err(|_| JobError::Assign(last_error()))?;
            }
        }
        self.assigned_pids.push(pid);
        Ok(())
    }

    pub fn terminate(&self) -> Result<(), JobError> {
        #[cfg(windows)]
        {
            use windows::Win32::System::JobObjects::TerminateJobObject;
            unsafe {
                TerminateJobObject(self.handle, 1).map_err(|_| JobError::Terminate(last_error()))?;
            }
        }
        Ok(())
    }
}

impl Drop for InstanceJob {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
