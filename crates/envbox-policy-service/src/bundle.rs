//! Protected installed bundle leases. No environment override or per-user
//! staging path participates in privileged Runtime selection.
use envbox_launcher::launcher::win::SafeHandle;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use windows::{
    core::PCWSTR,
    Win32::{Foundation::*, Storage::FileSystem::*},
};

type Result<T> = std::result::Result<T, String>;
pub struct InstalledBundle {
    pub(crate) root: PathBuf,
    managers: Vec<(PathBuf, [u8; 32])>,
    pub(crate) runtime_bundle: envbox_launcher::service_start::TrustedRuntimeBundle,
    _leases: Vec<SafeHandle>,
}
fn digest(file: HANDLE) -> Result<[u8; 32]> {
    unsafe {
        SetFilePointerEx(file, 0, None, FILE_BEGIN).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            let mut n = 0;
            ReadFile(file, Some(&mut buf), Some(&mut n), None).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n as usize]);
        }
        SetFilePointerEx(file, 0, None, FILE_BEGIN).map_err(|e| e.to_string())?;
        Ok(hash.finalize().into())
    }
}
fn lease(path: &Path, directory: bool) -> Result<SafeHandle> {
    envbox_launcher::service_start::lease_protected_path(path, directory)
}
impl InstalledBundle {
    pub(crate) fn lease_input_path(path: &Path, directory: bool) -> Result<Vec<SafeHandle>> {
        crate::protocol::validate_local_path(path)?;
        use std::os::windows::ffi::OsStrExt;
        let text = path.to_string_lossy();
        let drive = format!("{}\\", &text[..2]);
        let drive: Vec<u16> = drive.encode_utf16().chain(Some(0)).collect();
        unsafe {
            if GetDriveTypeW(PCWSTR(drive.as_ptr())) != 3 {
                return Err("local fixed-drive target required".into());
            } // DRIVE_FIXED
            let mut ancestry: Vec<_> = path.ancestors().filter(|p| p.parent().is_some()).collect();
            ancestry.reverse();
            let mut leases = vec![];
            for item in ancestry {
                let wide: Vec<u16> = item.as_os_str().encode_wide().chain(Some(0)).collect();
                let is_directory = item != path || directory;
                let flags = FILE_FLAG_OPEN_REPARSE_POINT
                    | if is_directory {
                        FILE_FLAG_BACKUP_SEMANTICS
                    } else {
                        FILE_FLAGS_AND_ATTRIBUTES(0)
                    };
                let file = SafeHandle(
                    CreateFileW(
                        PCWSTR(wide.as_ptr()),
                        FILE_READ_ATTRIBUTES.0,
                        FILE_SHARE_READ,
                        None,
                        OPEN_EXISTING,
                        flags,
                        None,
                    )
                    .map_err(|e| e.to_string())?,
                );
                let mut info = BY_HANDLE_FILE_INFORMATION::default();
                GetFileInformationByHandle(file.0, &mut info).map_err(|e| e.to_string())?;
                if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
                    || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != is_directory
                {
                    return Err("input path reparse point or type rejected".into());
                }
                leases.push(file);
            }
            Ok(leases)
        }
    }
    pub(crate) fn assert_target_unmanaged(&self, target: &Path) -> Result<Vec<SafeHandle>> {
        let normalized = target.to_string_lossy().to_ascii_uppercase();
        let root = std::fs::canonicalize(&self.root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_ascii_uppercase();
        if normalized == root || normalized.starts_with(&(root + "\\")) {
            return Err("installed management bundle cannot be a target".into());
        }
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let file = SafeHandle(
                CreateFileW(
                    PCWSTR(path.as_ptr()),
                    FILE_READ_ATTRIBUTES.0,
                    FILE_SHARE_READ,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    None,
                )
                .map_err(|e| e.to_string())?,
            );
            let mut t = BY_HANDLE_FILE_INFORMATION::default();
            GetFileInformationByHandle(file.0, &mut t).map_err(|e| e.to_string())?;
            if t.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_DIRECTORY.0)
                != 0
            {
                return Err("invalid target file".into());
            }
            for lease in &self._leases {
                let mut b = BY_HANDLE_FILE_INFORMATION::default();
                GetFileInformationByHandle(lease.0, &mut b).map_err(|e| e.to_string())?;
                if (t.dwVolumeSerialNumber, t.nFileIndexHigh, t.nFileIndexLow)
                    == (b.dwVolumeSerialNumber, b.nFileIndexHigh, b.nFileIndexLow)
                {
                    return Err("management bundle hardlink cannot be a target".into());
                }
            }
            let mut leases = vec![file];
            for ancestor in target
                .parent()
                .ok_or("target has no parent")?
                .ancestors()
                .filter(|p| p.parent().is_some())
            {
                let text: Vec<u16> = ancestor.as_os_str().encode_wide().chain(Some(0)).collect();
                let directory = SafeHandle(
                    CreateFileW(
                        PCWSTR(text.as_ptr()),
                        FILE_READ_ATTRIBUTES.0,
                        FILE_SHARE_READ,
                        None,
                        OPEN_EXISTING,
                        FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                        None,
                    )
                    .map_err(|e| e.to_string())?,
                );
                let mut info = BY_HANDLE_FILE_INFORMATION::default();
                GetFileInformationByHandle(directory.0, &mut info).map_err(|e| e.to_string())?;
                if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                    return Err("target ancestor reparse point rejected".into());
                }
                leases.push(directory);
            }
            // Reopen after ancestor leases establish the complete path; an
            // earlier rename/replacement must not be confused with the lease.
            let current = SafeHandle(
                CreateFileW(
                    PCWSTR(path.as_ptr()),
                    FILE_READ_ATTRIBUTES.0,
                    FILE_SHARE_READ,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    None,
                )
                .map_err(|e| e.to_string())?,
            );
            let mut c = BY_HANDLE_FILE_INFORMATION::default();
            GetFileInformationByHandle(current.0, &mut c).map_err(|e| e.to_string())?;
            if (t.dwVolumeSerialNumber, t.nFileIndexHigh, t.nFileIndexLow)
                != (c.dwVolumeSerialNumber, c.nFileIndexHigh, c.nFileIndexLow)
            {
                return Err("target path changed during validation".into());
            }
            leases.push(current);
            return Ok(leases);
        }
    }
    pub(crate) fn approve_manager(&self, process: HANDLE) -> Result<()> {
        unsafe {
            let image = management_image_path(process)?;
            let (manager, manager_digest) = self
                .managers
                .iter()
                .find(|(path, _)| {
                    image
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&path.to_string_lossy())
                })
                .ok_or("unapproved management executable")?;
            // The approved image and protected Runtime files remain leased for
            // the engine's lifetime; ordinary users cannot replace them.
            let observed = lease(&image, false)?;
            let approved = lease(manager, false)?;
            let mut a = BY_HANDLE_FILE_INFORMATION::default();
            let mut b = a;
            GetFileInformationByHandle(observed.0, &mut a).map_err(|e| e.to_string())?;
            GetFileInformationByHandle(approved.0, &mut b).map_err(|e| e.to_string())?;
            if (a.dwVolumeSerialNumber, a.nFileIndexHigh, a.nFileIndexLow)
                != (b.dwVolumeSerialNumber, b.nFileIndexHigh, b.nFileIndexLow)
            {
                return Err("management image identity changed".into());
            }
            if &digest(observed.0)? != manager_digest {
                return Err("management image digest changed".into());
            }
            Ok(())
        }
    }
    /// Root is derived from the service executable, not request/environment.
    pub fn from_service_executable() -> Result<Self> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        // Only the OS-derived service image may arrive with the extended DOS
        // prefix. Client paths never pass through this conversion.
        let image = executable.to_str().ok_or("non-Unicode service image")?;
        let executable = PathBuf::from(image.strip_prefix(r"\\?\").unwrap_or(image));
        crate::protocol::validate_local_path(&executable)?;
        let root = executable
            .parent()
            .ok_or("service executable has no directory")?
            .to_path_buf();
        let runtime_bundle = envbox_launcher::service_start::TrustedRuntimeBundle::open(&root)
            .map_err(|e| e.to_string())?;
        if root.as_os_str().to_string_lossy().starts_with(r"\\") {
            return Err("remote installed bundle rejected".into());
        }
        let mut leases = vec![];
        for ancestor in root.ancestors().filter(|p| p.parent().is_some()) {
            leases.push(lease(ancestor, true)?);
        }
        leases.push(lease(&executable, false)?);
        let runtime64 = root.join("envbox-runtime64.dll");
        let runtime32 = root.join("envbox-runtime32.dll");
        let mut managers = vec![];
        for name in ["aura.exe", "envbox.exe"] {
            let path = root.join(name);
            match std::fs::symlink_metadata(&path) {
                Ok(_) => {
                    let file = lease(&path, false)?;
                    let hash = digest(file.0)?;
                    leases.push(file);
                    managers.push((path, hash));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if managers.is_empty() {
            return Err("no approved installed management executable".into());
        }
        for file in [&runtime64, &runtime32] {
            leases.push(lease(file, false)?);
        }
        Ok(Self {
            root,
            managers,
            runtime_bundle,
            _leases: leases,
        })
    }
}

pub(crate) fn management_image_path(process: HANDLE) -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::{
        core::PWSTR,
        Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32},
    };
    unsafe {
        let mut text = vec![0u16; 32768];
        let mut size = text.len() as u32;
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(text.as_mut_ptr()),
            &mut size,
        )
        .map_err(|e| e.to_string())?;
        Ok(PathBuf::from(std::ffi::OsString::from_wide(
            &text[..size as usize],
        )))
    }
}
