//! Runtime attach seam (packaged-v1 ticket 40).
//!
//! One [`RuntimeInjector`] shared by PreExecution / PostActivation strategies.
//! Detours / remote-load stay here; Runtime C++ hooks are untouched.

use envbox_core::AttachStrategy;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AttachError {
    #[error("runtime DLL not found: {0}")]
    RuntimeDllMissing(String),
    #[error("injection unsupported: {0}")]
    Unsupported(String),
    #[error("inject into pid={pid} failed: {message}")]
    InjectFailed { pid: u32, message: String },
    #[error("process open failed (GetLastError={0})")]
    OpenProcess(u32),
}

/// Result of a successful runtime attach.
#[derive(Debug, Clone)]
pub struct AttachedRuntime {
    pub pid: u32,
    pub strategy: AttachStrategy,
    pub runtime_dll: PathBuf,
    /// Runtime reported ready (IPC RUNTIME_READY or ENVBOX_RUNTIME_LOADED).
    pub handshake_ok: bool,
}

/// Shared injector: load envbox-runtime into a PID regardless of create path.
pub struct RuntimeInjector {
    pub runtime_dll: PathBuf,
}

impl RuntimeInjector {
    pub fn new(runtime_dll: PathBuf) -> Self {
        Self { runtime_dll }
    }

    /// Resolve injector for a target PE (arch-matched DLL).
    pub fn for_target(target: &Path) -> Result<Self, AttachError> {
        let dll = crate::injection::resolve_runtime_dll_for_target(target)
            .map_err(|e| AttachError::RuntimeDllMissing(e.to_string()))?;
        Ok(Self::new(dll))
    }

    /// Attach by strategy.
    ///
    /// * PreExecution: process is suspended and may already carry the DLL via
    ///   DetourCreateProcessWithDllExW — this verifies / completes injection.
    /// * PostActivation: remote-load into an already-running PID.
    pub fn attach(
        &self,
        pid: u32,
        strategy: AttachStrategy,
        already_injected: bool,
    ) -> Result<AttachedRuntime, AttachError> {
        match strategy {
            AttachStrategy::PreExecution | AttachStrategy::PackageDebug => {
                if already_injected {
                    Ok(AttachedRuntime {
                        pid,
                        strategy,
                        runtime_dll: self.runtime_dll.clone(),
                        handshake_ok: handshake_probe(pid),
                    })
                } else {
                    self.inject_remote(pid)?;
                    Ok(AttachedRuntime {
                        pid,
                        strategy,
                        runtime_dll: self.runtime_dll.clone(),
                        handshake_ok: handshake_probe(pid),
                    })
                }
            }
            AttachStrategy::PostActivation => {
                self.inject_remote(pid)?;
                Ok(AttachedRuntime {
                    pid,
                    strategy,
                    runtime_dll: self.runtime_dll.clone(),
                    handshake_ok: handshake_probe(pid),
                })
            }
        }
    }

    /// Remote LoadLibrary into a live PID (PostActivation / late PreExecution).
    pub fn inject_remote(&self, pid: u32) -> Result<(), AttachError> {
        #[cfg(windows)]
        {
            win_inject_remote(pid, &self.runtime_dll)
        }
        #[cfg(not(windows))]
        {
            let _ = (pid, &self.runtime_dll);
            Err(AttachError::Unsupported(
                "remote inject is Windows-only".into(),
            ))
        }
    }
}

/// Best-effort handshake: Runtime sets ENVBOX_RUNTIME_LOADED after init.
fn handshake_probe(_pid: u32) -> bool {
    // V1: probe is process-local; remote marker check is optional.
    // IPC RUNTIME_READY is preferred (see ipc.rs). Treat as not-yet-confirmed.
    false
}

#[cfg(windows)]
fn win_inject_remote(pid: u32, dll: &Path) -> Result<(), AttachError> {
    use crate::launcher::win::SafeHandle;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{GetLastError, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
    };
    use windows::Win32::System::Threading::{
        CreateRemoteThread, OpenProcess, WaitForSingleObject, INFINITE, PROCESS_CREATE_THREAD,
        PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
    };
    use windows::core::PCWSTR;

    /// Frees remote allocation when the inject path returns early or finishes.
    struct RemoteAlloc {
        process: HANDLE,
        ptr: *mut core::ffi::c_void,
    }

    impl Drop for RemoteAlloc {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                unsafe {
                    let _ = VirtualFreeEx(self.process, self.ptr, 0, MEM_RELEASE);
                }
            }
        }
    }

    unsafe {
        let access = PROCESS_CREATE_THREAD
            | PROCESS_QUERY_INFORMATION
            | PROCESS_VM_OPERATION
            | PROCESS_VM_WRITE
            | PROCESS_VM_READ;
        let process = OpenProcess(access, false, pid)
            .map(SafeHandle)
            .map_err(|_| AttachError::OpenProcess(GetLastError().0))?;

        let wide: Vec<u16> = dll
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let bytes = wide.len() * 2;
        let remote = VirtualAllocEx(
            process.0,
            None,
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote.is_null() {
            return Err(AttachError::InjectFailed {
                pid,
                message: format!("VirtualAllocEx failed ({})", GetLastError().0),
            });
        }
        let remote = RemoteAlloc {
            process: process.0,
            ptr: remote,
        };

        let mut written = 0usize;
        if WriteProcessMemory(
            process.0,
            remote.ptr,
            wide.as_ptr() as *const _,
            bytes,
            Some(&mut written),
        )
        .is_err()
        {
            return Err(AttachError::InjectFailed {
                pid,
                message: format!("WriteProcessMemory failed ({})", GetLastError().0),
            });
        }

        let k32 = GetModuleHandleW(PCWSTR::from_raw(windows::core::w!("kernel32.dll").as_ptr()))
            .map_err(|_| AttachError::InjectFailed {
                pid,
                message: format!("GetModuleHandleW(kernel32) failed ({})", GetLastError().0),
            })?;
        let load_library = GetProcAddress(k32, windows::core::s!("LoadLibraryW"))
            .ok_or_else(|| AttachError::InjectFailed {
                pid,
                message: format!("GetProcAddress(LoadLibraryW) failed ({})", GetLastError().0),
            })?;

        let thread = CreateRemoteThread(
            process.0,
            None,
            0,
            Some(std::mem::transmute(load_library)),
            Some(remote.ptr),
            0,
            None,
        )
        .map(SafeHandle)
        .map_err(|e| AttachError::InjectFailed {
            pid,
            message: format!("CreateRemoteThread failed: {e}"),
        })?;

        let wait = WaitForSingleObject(thread.0, INFINITE);
        // Drop order: thread → remote → process (all RAII).
        drop(thread);
        drop(remote);
        drop(process);
        if wait != WAIT_OBJECT_0 {
            return Err(AttachError::InjectFailed {
                pid,
                message: "WaitForSingleObject on remote thread failed".into(),
            });
        }
        Ok(())
    }
}

/// Attach seam used by EnvironmentSession (ticket 37).
pub trait RuntimeAttacher {
    fn attach(
        &self,
        pid: u32,
        strategy: AttachStrategy,
        already_injected: bool,
    ) -> Result<AttachedRuntime, AttachError>;
}

impl RuntimeAttacher for RuntimeInjector {
    fn attach(
        &self,
        pid: u32,
        strategy: AttachStrategy,
        already_injected: bool,
    ) -> Result<AttachedRuntime, AttachError> {
        RuntimeInjector::attach(self, pid, strategy, already_injected)
    }
}

/// Pick attach strategy from capabilities (delegates to core policy).
pub fn select_strategy(
    caps: &envbox_core::TargetCapabilities,
) -> Option<AttachStrategy> {
    envbox_core::select_attach_strategy(caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::{AttachStrategy, TargetCapabilities};

    #[test]
    fn pre_execution_when_can_suspend() {
        let caps = TargetCapabilities::win32();
        assert_eq!(
            select_strategy(&caps),
            Some(AttachStrategy::PreExecution)
        );
    }

    #[test]
    fn post_activation_when_cannot_suspend() {
        let caps = TargetCapabilities {
            can_suspend: false,
            can_inject_runtime: true,
            can_create_environment_block: false,
            can_assign_job: true,
            can_track_children: true,
        };
        assert_eq!(
            select_strategy(&caps),
            Some(AttachStrategy::PostActivation)
        );
    }
}
