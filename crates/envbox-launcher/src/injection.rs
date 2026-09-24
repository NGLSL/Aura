//! Locate envbox-runtime DLL and create the process with Detours injection.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("runtime DLL not found (set ENVBOX_RUNTIME_DLL or place envbox-runtime64.dll beside envbox.exe)")]
    RuntimeDllMissing,
    #[error("runtime DLL path is not a file: {0}")]
    RuntimeDllInvalid(PathBuf),
    #[error("runtime DLL path encode failed (GetLastError={1}): {0}")]
    RuntimeDllPathEncode(PathBuf, u32),
    #[error("DetourCreateProcessWithDllExW failed (GetLastError={0})")]
    DetourCreateProcess(u32),
}

/// Resolve `envbox-runtime64.dll` / `envbox-runtime32.dll` (x64 preferred).
/// Order: `ENVBOX_RUNTIME_DLL` → beside current exe → target/debug from deps.
pub fn resolve_runtime_dll() -> Result<PathBuf, InjectError> {
    if let Ok(explicit) = std::env::var("ENVBOX_RUNTIME_DLL") {
        let p = PathBuf::from(explicit);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(InjectError::RuntimeDllInvalid(p))
        };
    }

    // Prefer matching pointer width (V0.1 ships x64 first).
    let names: &[&str] = if cfg!(target_pointer_width = "64") {
        &["envbox-runtime64.dll", "envbox-runtime32.dll"]
    } else {
        &["envbox-runtime32.dll", "envbox-runtime64.dll"]
    };

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
            // cargo test binaries live in target/debug/deps — also check target/debug.
            if parent.file_name().and_then(|s| s.to_str()) == Some("deps") {
                if let Some(grand) = parent.parent() {
                    dirs.push(grand.to_path_buf());
                }
            }
        }
    }

    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(InjectError::RuntimeDllMissing)
}

/// ANSI path for Detours `lpDllName` (LPCSTR even on the W API).
/// Uses the ANSI code page via WideCharToMultiByte(CP_ACP).
pub fn dll_path_ansi(path: &Path) -> Result<Vec<u8>, InjectError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{GetLastError, BOOL};
    use windows::Win32::Globalization::{WideCharToMultiByte, CP_ACP};

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.is_empty() {
        return Err(InjectError::RuntimeDllInvalid(path.to_path_buf()));
    }
    let mut ansi = vec![0u8; wide.len() * 2 + 2];
    let mut used_default = BOOL(0);
    let n = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            &wide,
            Some(&mut ansi),
            None,
            Some(&mut used_default),
        )
    };
    if n <= 0 {
        let code = unsafe { GetLastError().0 };
        return Err(InjectError::RuntimeDllPathEncode(path.to_path_buf(), code));
    }
    ansi.truncate(n as usize);
    ansi.push(0);
    Ok(ansi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_rejected() {
        let err = dll_path_ansi(Path::new("")).unwrap_err();
        assert!(matches!(err, InjectError::RuntimeDllInvalid(_)));
    }

    #[test]
    fn dll_path_ansi_encodes_ascii() {
        let bytes = dll_path_ansi(Path::new(r"C:\x\envbox-runtime64.dll")).unwrap();
        let s = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(s, r"C:\x\envbox-runtime64.dll");
    }

    #[test]
    fn arch_prefers_matching_bitness_name() {
        let names: &[&str] = if cfg!(target_pointer_width = "64") {
            &["envbox-runtime64.dll", "envbox-runtime32.dll"]
        } else {
            &["envbox-runtime32.dll", "envbox-runtime64.dll"]
        };
        assert!(names[0].contains("64") || names[0].contains("32"));
        if cfg!(target_pointer_width = "64") {
            assert!(names[0].contains("64"));
        }
    }
}
