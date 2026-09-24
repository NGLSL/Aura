//! Locate envbox-runtime DLL and create the process with Detours injection.
//!
//! Ticket 30: elevation/integrity failures map to an explicit Startup Fail Policy
//! error (never silent unvirtualized launch).
//! Ticket 31: resolve Runtime DLL to the target PE architecture; mismatch never
//! silently falls back to the other bitness.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Win32 error codes that indicate integrity/elevation blocked process creation
/// or injection (ticket 30).
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_BAD_EXE_FORMAT: u32 = 193;
pub const ERROR_ELEVATION_REQUIRED: u32 = 740;
pub const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("runtime DLL not found (set ENVBOX_RUNTIME_DLL or place envbox-runtime{expected} beside envbox.exe)")]
    RuntimeDllMissing {
        /// Preferred `32` / `64` suffix for the target architecture.
        expected: &'static str,
    },
    #[error("runtime DLL path is not a file: {0}")]
    RuntimeDllInvalid(PathBuf),
    #[error("runtime DLL path encode failed (GetLastError={1}): {0}")]
    RuntimeDllPathEncode(PathBuf, u32),
    #[error("DetourCreateProcessWithDllExW failed (GetLastError={0})")]
    DetourCreateProcess(u32),
    #[error(
        "integrity/elevation or access denied during inject (GetLastError={0}); \
         the target must run at the same integrity level as EnvBox or lower \
         (start EnvBox elevated if the target requires elevation); \
         also check path ACLs / antivirus / Job restrictions"
    )]
    ElevationIntegrity(u32),
    #[error(
        "architecture mismatch: runtime DLL is {dll_arch} but target is {target_arch} \
         ({dll} vs {target}); refusing silent fallback (Startup Fail Policy)"
    )]
    ArchitectureMismatch {
        dll: PathBuf,
        dll_arch: PeArch,
        target: PathBuf,
        target_arch: PeArch,
    },
    #[error(
        "target/runtime architecture load failed (GetLastError={0}); \
         use a matching envbox-runtime32.dll / envbox-runtime64.dll (Startup Fail Policy)"
    )]
    ArchitectureLoad(u32),
    #[error("cannot read PE architecture of {0} (os={1})")]
    PeUnreadable(PathBuf, i32),
}

/// IMAGE_FILE_MACHINE values we care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeArch {
    X86,
    X64,
    Unknown(u16),
}

impl std::fmt::Display for PeArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeArch::X86 => write!(f, "x86"),
            PeArch::X64 => write!(f, "x64"),
            PeArch::Unknown(m) => write!(f, "unknown(0x{m:04x})"),
        }
    }
}

const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

/// Read `IMAGE_FILE_HEADER.Machine` from a PE image (DOS stub → PE signature).
pub fn pe_arch(path: &Path) -> Result<PeArch, InjectError> {
    use std::io::{Read, Seek, SeekFrom};

    let pe_unreadable = |err: &std::io::Error| {
        InjectError::PeUnreadable(path.to_path_buf(), err.raw_os_error().unwrap_or(0))
    };

    let mut f = std::fs::File::open(path).map_err(|e| pe_unreadable(&e))?;
    let mut dos = [0u8; 0x40];
    f.read_exact(&mut dos).map_err(|e| pe_unreadable(&e))?;
    if &dos[0..2] != b"MZ" {
        return Err(InjectError::PeUnreadable(
            path.to_path_buf(),
            ERROR_BAD_EXE_FORMAT as i32,
        ));
    }
    let lfanew = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as u64;
    f.seek(SeekFrom::Start(lfanew))
        .map_err(|e| pe_unreadable(&e))?;
    let mut pe = [0u8; 6];
    f.read_exact(&mut pe).map_err(|e| pe_unreadable(&e))?;
    if &pe[0..4] != b"PE\0\0" {
        return Err(InjectError::PeUnreadable(
            path.to_path_buf(),
            ERROR_BAD_EXE_FORMAT as i32,
        ));
    }
    let machine = u16::from_le_bytes([pe[4], pe[5]]);
    Ok(match machine {
        IMAGE_FILE_MACHINE_I386 => PeArch::X86,
        IMAGE_FILE_MACHINE_AMD64 => PeArch::X64,
        other => PeArch::Unknown(other),
    })
}

/// Map a Win32 failure from CreateProcess / Detours to a typed inject error
/// (ticket 30 elevation/integrity, ticket 31 bad-exe-format).
pub fn map_create_process_error(code: u32) -> InjectError {
    match code {
        ERROR_ELEVATION_REQUIRED | ERROR_ACCESS_DENIED | ERROR_PRIVILEGE_NOT_HELD => {
            InjectError::ElevationIntegrity(code)
        }
        ERROR_BAD_EXE_FORMAT => InjectError::ArchitectureLoad(code),
        _ => InjectError::DetourCreateProcess(code),
    }
}

/// True when the error means the target could not be injected because of
/// integrity/elevation (ticket 30).
pub fn is_elevation_integrity(err: &InjectError) -> bool {
    matches!(err, InjectError::ElevationIntegrity(_))
}

fn arch_suffix(arch: PeArch) -> &'static str {
    match arch {
        PeArch::X86 => "32",
        PeArch::X64 => "64",
        PeArch::Unknown(_) => "64",
    }
}

/// Resolve `envbox-runtime64.dll` / `envbox-runtime32.dll` for the **host** pointer
/// width (no target PE). Prefer `resolve_runtime_dll_for_target`.
/// Order: `ENVBOX_RUNTIME_DLL` → beside current exe → target/debug from deps.
pub fn resolve_runtime_dll() -> Result<PathBuf, InjectError> {
    let host: PeArch = if cfg!(target_pointer_width = "64") {
        PeArch::X64
    } else {
        PeArch::X86
    };
    resolve_runtime_dll_for_arch(host, None)
}

/// Resolve the Runtime DLL that matches `target` PE architecture (ticket 31).
///
/// * Explicit `ENVBOX_RUNTIME_DLL` must match the target bitness — mismatch is a
///   hard error (no silent swap).
/// * Name search only accepts the matching `envbox-runtime32/64.dll`.
pub fn resolve_runtime_dll_for_target(target: &Path) -> Result<PathBuf, InjectError> {
    let target_arch = pe_arch(target)?;
    resolve_runtime_dll_for_arch(target_arch, Some(target))
}

fn resolve_runtime_dll_for_arch(
    target_arch: PeArch,
    target_path: Option<&Path>,
) -> Result<PathBuf, InjectError> {
    if let Ok(explicit) = std::env::var("ENVBOX_RUNTIME_DLL") {
        let p = PathBuf::from(explicit);
        if !p.is_file() {
            return Err(InjectError::RuntimeDllInvalid(p));
        }
        let dll_arch = pe_arch(&p).unwrap_or(PeArch::Unknown(0));
        match (dll_arch, target_arch) {
            (PeArch::X86, PeArch::X86) | (PeArch::X64, PeArch::X64) => Ok(p),
            // Unknown PE (corrupt / non-PE): let Detours fail closed later.
            (PeArch::Unknown(_), _) => Ok(p),
            _ => Err(InjectError::ArchitectureMismatch {
                dll: p,
                dll_arch,
                target: target_path.map(|t| t.to_path_buf()).unwrap_or_default(),
                target_arch,
            }),
        }
    } else {
        // Only the matching bitness name — never silently fall back (ticket 31).
        let names: &[&str] = match target_arch {
            PeArch::X86 => &["envbox-runtime32.dll"],
            PeArch::X64 => &["envbox-runtime64.dll"],
            PeArch::Unknown(_) => &["envbox-runtime64.dll", "envbox-runtime32.dll"],
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

        Err(InjectError::RuntimeDllMissing {
            expected: arch_suffix(target_arch),
        })
    }
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

    fn write_fake_pe(path: &Path, machine: u16) {
        // Minimal DOS stub + PE signature + IMAGE_FILE_HEADER.Machine.
        let mut buf = vec![0u8; 0x80];
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[0x3c] = 0x40; // e_lfanew
        buf[0x40] = b'P';
        buf[0x41] = b'E';
        buf[0x42] = 0;
        buf[0x43] = 0;
        buf[0x44] = (machine & 0xff) as u8;
        buf[0x45] = (machine >> 8) as u8;
        std::fs::write(path, buf).unwrap();
    }

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

    #[test]
    fn pe_arch_reads_i386_and_amd64() {
        let dir = std::env::temp_dir().join(format!("envbox-pe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let x86 = dir.join("a.dll");
        let x64 = dir.join("b.dll");
        write_fake_pe(&x86, IMAGE_FILE_MACHINE_I386);
        write_fake_pe(&x64, IMAGE_FILE_MACHINE_AMD64);
        assert_eq!(pe_arch(&x86).unwrap(), PeArch::X86);
        assert_eq!(pe_arch(&x64).unwrap(), PeArch::X64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ticket 30: elevation/integrity error codes map to a clear message that
    /// names integrity/elevation and the required level relationship.
    #[test]
    fn elevation_errors_map_with_integrity_guidance() {
        for code in [
            ERROR_ELEVATION_REQUIRED,
            ERROR_ACCESS_DENIED,
            ERROR_PRIVILEGE_NOT_HELD,
        ] {
            let err = map_create_process_error(code);
            assert!(
                is_elevation_integrity(&err),
                "code {code} must map to elevation/integrity: {err}"
            );
            let msg = err.to_string();
            assert!(
                msg.contains("integrity/elevation"),
                "message must contain integrity/elevation: {msg}"
            );
            assert!(
                msg.contains("same integrity level as EnvBox or lower"),
                "message must state required integrity: {msg}"
            );
            // ACCESS_DENIED is broader than elevation — message must not overclaim.
            assert!(
                msg.contains("access denied") || msg.contains("integrity"),
                "message should acknowledge access/integrity breadth: {msg}"
            );
        }
    }

    #[test]
    fn bad_exe_format_maps_to_architecture_load() {
        let err = map_create_process_error(ERROR_BAD_EXE_FORMAT);
        assert!(matches!(err, InjectError::ArchitectureLoad(_)));
        assert!(err.to_string().contains("architecture"));
    }

    #[test]
    fn other_errors_stay_detour_create_process() {
        let err = map_create_process_error(2); // ERROR_FILE_NOT_FOUND
        assert!(matches!(err, InjectError::DetourCreateProcess(2)));
    }

    /// Ticket 31: explicit Runtime DLL must match target bitness.
    /// Wrong arch → hard error (no silent fallback); matching arch → accepted.
    /// Serial in one test to avoid racing on `ENVBOX_RUNTIME_DLL`.
    #[test]
    fn explicit_runtime_dll_arch_mismatch_is_hard_error_and_match_ok() {
        let dir = std::env::temp_dir().join(format!("envbox-arch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let x86 = dir.join("envbox-runtime32.dll");
        let x64 = dir.join("envbox-runtime64.dll");
        write_fake_pe(&x86, IMAGE_FILE_MACHINE_I386);
        write_fake_pe(&x64, IMAGE_FILE_MACHINE_AMD64);

        std::env::set_var("ENVBOX_RUNTIME_DLL", &x86);
        let err = resolve_runtime_dll_for_arch(PeArch::X64, Some(Path::new(r"C:\app\target.exe")))
            .unwrap_err();
        assert!(
            matches!(err, InjectError::ArchitectureMismatch { .. }),
            "got {err}"
        );
        if let InjectError::ArchitectureMismatch { target, .. } = &err {
            assert_eq!(target, Path::new(r"C:\app\target.exe"));
        }
        let msg = err.to_string();
        assert!(msg.contains("architecture mismatch"), "{msg}");
        assert!(msg.contains("refusing silent fallback"), "{msg}");

        std::env::set_var("ENVBOX_RUNTIME_DLL", &x64);
        let got = resolve_runtime_dll_for_arch(PeArch::X64, None).unwrap();
        assert_eq!(got, x64);

        std::env::remove_var("ENVBOX_RUNTIME_DLL");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
