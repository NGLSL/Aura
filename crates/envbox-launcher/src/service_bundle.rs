//! Stable protected installed Runtime selection. Constructors enforce ownership,
//! effective write ACL, path ancestry and Runtime capabilities before issuing a lease.
use crate::launcher::win::SafeHandle;
use crate::SessionError;
use std::{
    ffi::c_void,
    path::{Path, PathBuf},
};
use windows::{
    core::PCWSTR,
    Win32::{Foundation::*, Security::Authorization::*, Security::*, Storage::FileSystem::*},
};
type Result<T> = std::result::Result<T, String>;
fn sid_bytes(sid: PSID) -> Result<Vec<u8>> {
    unsafe {
        if !IsValidSid(sid).as_bool() {
            return Err("invalid ACL SID".into());
        }
        let len = GetLengthSid(sid) as usize;
        if len == 0 || len > 1024 {
            return Err("invalid ACL SID length".into());
        }
        Ok(std::slice::from_raw_parts(sid.0.cast::<u8>(), len).to_vec())
    }
}
fn privileged(sid: &[u8]) -> bool {
    if sid == [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0]
        || sid == [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0]
    {
        return true;
    }
    // Read-only `sc.exe showsid TrustedInstaller` canonical service SID.
    let mut installer = vec![1, 6, 0, 0, 0, 0, 0, 5];
    for rid in [
        80u32, 956008885, 3418522649, 1831038044, 1853292631, 2271478464,
    ] {
        installer.extend_from_slice(&rid.to_le_bytes());
    }
    sid == installer
}
pub fn lease_protected_path(path: &Path, directory: bool) -> Result<SafeHandle> {
    use std::os::windows::ffi::OsStrExt;
    let text: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let flags = FILE_FLAG_OPEN_REPARSE_POINT
            | if directory {
                FILE_FLAG_BACKUP_SEMANTICS
            } else {
                FILE_FLAGS_AND_ATTRIBUTES(0)
            };
        let file = SafeHandle(
            CreateFileW(
                PCWSTR(text.as_ptr()),
                0x00020080 | u32::from(!directory),
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
            || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
        {
            return Err("bundle file type or reparse point rejected".into());
        }
        let mut owner = PSID::default();
        let mut acl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(
            file.0,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut acl),
            None,
            Some(&mut descriptor),
        )
        .ok()
        .map_err(|e| e.to_string())?;
        struct Descriptor(PSECURITY_DESCRIPTOR);
        impl Drop for Descriptor {
            fn drop(&mut self) {
                unsafe {
                    let _ = LocalFree(HLOCAL(self.0 .0));
                }
            }
        }
        let _descriptor = Descriptor(descriptor);
        if !privileged(&sid_bytes(owner)?) || acl.is_null() {
            return Err("bundle owner or unrestricted ACL rejected".into());
        }
        let count = (*acl).AceCount;
        for index in 0..count {
            let mut ace: *mut c_void = std::ptr::null_mut();
            GetAce(acl, index.into(), &mut ace).map_err(|e| e.to_string())?;
            let header = &*ace.cast::<ACE_HEADER>();
            if header.AceFlags & 0x08 != 0 {
                continue;
            } // inheritance-only, not effective on this object
            if header.AceType == 1 {
                continue;
            } // deny can only reduce access
            if header.AceType != 0 {
                return Err("unsupported installed bundle ACE".into());
            }
            let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid = sid_bytes(PSID(std::ptr::addr_of!(allowed.SidStart).cast_mut().cast()))?;
            // write/append/delete-child/attributes/delete/DACL/owner and generic writes
            // The volume root may grant create-directory without permission
            // to replace a pinned protected child (the default C:\ ACL does).
            // Every installed ancestor/leaf below that root rejects all creation
            // rights, so a user cannot plant a service sidecar DLL.
            let mutation_mask = if directory && path.parent().is_none() {
                0x500d0156 & !0x4
            } else {
                0x500d0156
            };
            if allowed.Mask & mutation_mask != 0 && !privileged(&sid) {
                return Err("installed bundle is writable by an untrusted principal".into());
            }
        }
        Ok(file)
    }
}

/// Possession proves the installed pair and its ancestors remain protected and
/// pinned. No public unchecked constructor or mutable path fields exist.
pub struct TrustedRuntimeBundle {
    runtime64: PathBuf,
    runtime32: PathBuf,
    _leases: Vec<SafeHandle>,
}
impl TrustedRuntimeBundle {
    pub fn open(root: &Path) -> std::result::Result<Self, SessionError> {
        let fail = |e: String| SessionError::Unsupported(format!("installed Runtime bundle: {e}"));
        validate_service_input_path(root).map_err(fail)?;
        let mut ancestors: Vec<_> = root.ancestors().collect();
        ancestors.reverse();
        let mut leases = Vec::new();
        for ancestor in ancestors {
            leases.push(lease_protected_path(ancestor, true).map_err(fail)?);
        }
        let runtime64 = root.join("envbox-runtime64.dll");
        let runtime32 = root.join("envbox-runtime32.dll");
        for (path, arch) in [
            (&runtime64, crate::PeArch::X64),
            (&runtime32, crate::PeArch::X86),
        ] {
            leases.push(lease_protected_path(path, false).map_err(fail)?);
            if crate::pe_arch(path).map_err(|e| fail(e.to_string()))? != arch {
                return Err(fail("Runtime architecture mismatch".into()));
            }
            crate::recovery::validate_trusted_service_bootstrap(path)
                .map_err(|e| fail(e.to_string()))?;
        }
        Ok(Self {
            runtime64,
            runtime32,
            _leases: leases,
        })
    }
    pub fn runtime64(&self) -> &Path {
        &self.runtime64
    }
    pub fn runtime32(&self) -> &Path {
        &self.runtime32
    }
    #[cfg(test)]
    pub(crate) fn fixture(runtime64: PathBuf, runtime32: PathBuf) -> Self {
        Self {
            runtime64,
            runtime32,
            _leases: vec![],
        }
    }
}

/// Only ordinary local fixed-drive DOS paths are accepted at the public seam.
/// Validation uses Windows UTF-16 and occurs before any filesystem access.
pub fn validate_service_input_path(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.contains(&0)
        || units.len() < 3
        || units[1] != b':' as u16
        || !((b'A' as u16..=b'Z' as u16).contains(&units[0])
            || (b'a' as u16..=b'z' as u16).contains(&units[0]))
        || ![b'\\' as u16, b'/' as u16].contains(&units[2])
    {
        return Err("ordinary absolute local DOS path required".into());
    }
    if units[3..]
        .split(|unit| *unit == b'\\' as u16 || *unit == b'/' as u16)
        .any(|part| part == [b'.' as u16] || part == [b'.' as u16, b'.' as u16])
    {
        return Err("raw dot path component rejected".into());
    }
    if path.components().any(|part| {
        matches!(part, Component::ParentDir)
            || matches!(part,Component::Prefix(p) if !matches!(p.kind(),Prefix::Disk(_)))
    }) {
        return Err("traversal or device path rejected".into());
    }
    // NTFS alternate streams and trailing-dot/space aliases are not accepted.
    if units[2..].contains(&(b':' as u16))
        || path.components().any(
            |c| matches!(c,Component::Normal(name) if name.to_string_lossy().ends_with(['.',' '])),
        )
    {
        return Err("aliased path rejected".into());
    }
    for part in path.components() {
        if let Component::Normal(name) = part {
            let text = name.to_string_lossy().to_ascii_uppercase();
            let base = text.split('.').next().unwrap_or("");
            if matches!(base, "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
                || ((base.starts_with("COM") || base.starts_with("LPT"))
                    && base.len() == 4
                    && matches!(base.as_bytes()[3], b'1'..=b'9'))
            {
                return Err("DOS device alias rejected".into());
            }
        }
    }
    let drive = [units[0], b':' as u16, b'\\' as u16, 0];
    if unsafe { GetDriveTypeW(PCWSTR(drive.as_ptr())) } != 3 {
        return Err("local fixed-drive path required".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_service_paths_reject_unsafe_forms_before_filesystem_access() {
        for path in [
            r"C:\safe\..\target.exe",
            r"\\server\share\target.exe",
            r"\\.\C:\target.exe",
            r"\\?\C:\target.exe",
            r"C:target.exe",
            r"C:\NUL",
            r"C:\folder\CON.exe",
            r"C:\target.exe:stream",
            r"C:\folder.\target.exe",
            r"C:\folder \target.exe",
            "C:\\file\0.exe",
        ] {
            assert!(
                validate_service_input_path(Path::new(path)).is_err(),
                "{path:?}"
            );
            assert!(
                TrustedRuntimeBundle::open(Path::new(path)).is_err(),
                "{path:?}"
            );
        }
    }
    #[test]
    fn protected_volume_root_allows_create_children_without_existing_child_mutation() {
        let _root = lease_protected_path(Path::new(r"C:\"), true).unwrap();
        let _program_files = lease_protected_path(Path::new(r"C:\Program Files"), true).unwrap();
    }
    #[test]
    fn ordinary_user_directory_cannot_issue_trusted_runtime_lease() {
        let directory =
            std::env::temp_dir().join(format!("aura-untrusted-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("envbox-runtime64.dll"), b"untrusted").unwrap();
        std::fs::write(directory.join("envbox-runtime32.dll"), b"untrusted").unwrap();
        assert!(TrustedRuntimeBundle::open(&directory).is_err());
        std::fs::remove_file(directory.join("envbox-runtime64.dll")).unwrap();
        std::fs::remove_file(directory.join("envbox-runtime32.dll")).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}
