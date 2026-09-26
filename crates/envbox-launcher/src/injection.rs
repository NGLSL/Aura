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
    #[error("cannot stage runtime DLL for detached instance ({source_path} -> {destination}): {message}")]
    RuntimeDllStage {
        source_path: PathBuf,
        destination: PathBuf,
        message: String,
    },
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

/// Copy the paired Runtime DLL products into a content-addressed cache outside
/// the install directory. A running instance can then survive Aura exit or an
/// installer upgrade without holding the installed DLLs open.
pub fn stage_runtime_dll(source: &Path, instance_id: uuid::Uuid) -> Result<PathBuf, InjectError> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    stage_runtime_dll_at(source, instance_id, &base)
}

fn stage_runtime_dll_at(
    source: &Path,
    instance_id: uuid::Uuid,
    base: &Path,
) -> Result<PathBuf, InjectError> {
    let selected_name = source
        .file_name()
        .ok_or_else(|| InjectError::RuntimeDllInvalid(source.to_path_buf()))?
        .to_os_string();
    let bundle = runtime_bundle(source, base)?;
    let cache_key = runtime_bundle_cache_key(&bundle);
    let cache_root = base.join("com.aura.envbox").join("runtime");
    let primary_dir = cache_root.join(&cache_key);

    if runtime_bundle_matches(&primary_dir, &bundle) {
        return Ok(primary_dir.join(selected_name));
    }

    // A partial directory containing only matching files is safe to complete.
    // An incompatible existing entry must never be overwritten while a running
    // instance may still have it loaded, so isolate this launch by instance id.
    let destination_dir = if runtime_bundle_conflicts(&primary_dir, &bundle) {
        cache_root.join(format!("{cache_key}-{instance_id}"))
    } else {
        primary_dir
    };
    std::fs::create_dir_all(&destination_dir).map_err(|err| InjectError::RuntimeDllStage {
        source_path: source.to_path_buf(),
        destination: destination_dir.clone(),
        message: err.to_string(),
    })?;

    for file in &bundle {
        stage_runtime_bundle_file(file, instance_id, &destination_dir)?;
    }
    if !runtime_bundle_matches(&destination_dir, &bundle) {
        return Err(InjectError::RuntimeDllStage {
            source_path: source.to_path_buf(),
            destination: destination_dir.clone(),
            message: "staged Runtime bundle fingerprint mismatch".into(),
        });
    }
    Ok(destination_dir.join(selected_name))
}

#[derive(Debug)]
struct RuntimeBundleFile {
    source: PathBuf,
    file_name: std::ffi::OsString,
    fingerprint: (u64, u64),
}

/// Detours switches `envbox-runtime64.dll` to `envbox-runtime32.dll` (and the
/// reverse) when a process creates a child of the other architecture. Both
/// products therefore have to remain together at the exact same directory.
fn runtime_bundle(
    source: &Path,
    error_destination: &Path,
) -> Result<Vec<RuntimeBundleFile>, InjectError> {
    let source_name = source
        .file_name()
        .ok_or_else(|| InjectError::RuntimeDllInvalid(source.to_path_buf()))?;
    let mut sources = vec![source.to_path_buf()];
    let source_name_text = source_name.to_string_lossy();
    let sibling_name = if source_name_text.eq_ignore_ascii_case("envbox-runtime64.dll") {
        Some("envbox-runtime32.dll")
    } else if source_name_text.eq_ignore_ascii_case("envbox-runtime32.dll") {
        Some("envbox-runtime64.dll")
    } else {
        None
    };
    if let (Some(parent), Some(sibling_name)) = (source.parent(), sibling_name) {
        let sibling = parent.join(sibling_name);
        if sibling.is_file() {
            sources.push(sibling);
        }
    }
    sources.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
    });

    sources
        .into_iter()
        .map(|source| {
            let file_name = source
                .file_name()
                .ok_or_else(|| InjectError::RuntimeDllInvalid(source.clone()))?
                .to_os_string();
            let fingerprint =
                runtime_fingerprint(&source).map_err(|err| InjectError::RuntimeDllStage {
                    source_path: source.clone(),
                    destination: error_destination.to_path_buf(),
                    message: err.to_string(),
                })?;
            Ok(RuntimeBundleFile {
                source,
                file_name,
                fingerprint,
            })
        })
        .collect()
}

fn runtime_bundle_cache_key(bundle: &[RuntimeBundleFile]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut total_length = 0u64;
    for file in bundle {
        let name = file.file_name.to_string_lossy().to_ascii_lowercase();
        for byte in name
            .as_bytes()
            .iter()
            .chain([0xff].iter())
            .chain(file.fingerprint.0.to_le_bytes().iter())
            .chain(file.fingerprint.1.to_le_bytes().iter())
        {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        total_length = total_length.wrapping_add(file.fingerprint.0);
    }
    format!("bundle-{}-{total_length:016x}-{hash:016x}", bundle.len())
}

fn runtime_bundle_matches(directory: &Path, bundle: &[RuntimeBundleFile]) -> bool {
    bundle.iter().all(|file| {
        let destination = directory.join(&file.file_name);
        destination.is_file() && runtime_fingerprint(&destination).ok() == Some(file.fingerprint)
    })
}

fn runtime_bundle_conflicts(directory: &Path, bundle: &[RuntimeBundleFile]) -> bool {
    bundle.iter().any(|file| {
        let destination = directory.join(&file.file_name);
        destination.exists()
            && (!destination.is_file()
                || runtime_fingerprint(&destination).ok() != Some(file.fingerprint))
    })
}

fn stage_runtime_bundle_file(
    file: &RuntimeBundleFile,
    instance_id: uuid::Uuid,
    directory: &Path,
) -> Result<(), InjectError> {
    let destination = directory.join(&file.file_name);
    if destination.is_file() && runtime_fingerprint(&destination).ok() == Some(file.fingerprint) {
        return Ok(());
    }
    let stage_error = |err: std::io::Error| InjectError::RuntimeDllStage {
        source_path: file.source.clone(),
        destination: destination.clone(),
        message: err.to_string(),
    };
    let temporary = directory.join(format!(
        ".{}-{}.tmp",
        file.file_name.to_string_lossy(),
        instance_id.simple()
    ));
    std::fs::copy(&file.source, &temporary).map_err(&stage_error)?;
    if runtime_fingerprint(&temporary).map_err(&stage_error)? != file.fingerprint {
        let _ = std::fs::remove_file(&temporary);
        return Err(InjectError::RuntimeDllStage {
            source_path: file.source.clone(),
            destination,
            message: "copy fingerprint mismatch".into(),
        });
    }
    if let Err(err) = std::fs::rename(&temporary, &destination) {
        if destination.is_file() && runtime_fingerprint(&destination).ok() == Some(file.fingerprint)
        {
            let _ = std::fs::remove_file(&temporary);
            return Ok(());
        }
        let _ = std::fs::remove_file(&temporary);
        return Err(stage_error(err));
    }
    Ok(())
}

fn runtime_fingerprint(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut length = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length += read as u64;
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok((length, hash))
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
            // Corrupt / non-PE Runtime must fail closed (Startup Fail Policy).
            (PeArch::Unknown(_), _) => Err(InjectError::RuntimeDllInvalid(p)),
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

    #[test]
    fn runtime_bundle_is_staged_outside_install_directory_and_reused_by_content() {
        let dir = std::env::temp_dir().join(format!("envbox-stage-{}", uuid::Uuid::new_v4()));
        let install = dir.join("install");
        let local = dir.join("local");
        std::fs::create_dir_all(&install).unwrap();
        let source64 = install.join("envbox-runtime64.dll");
        let source32 = install.join("envbox-runtime32.dll");
        std::fs::write(&source64, b"runtime-64").unwrap();
        std::fs::write(&source32, b"runtime-32").unwrap();

        let staged64 = stage_runtime_dll_at(&source64, uuid::Uuid::new_v4(), &local).unwrap();
        let staged32 = stage_runtime_dll_at(&source32, uuid::Uuid::new_v4(), &local).unwrap();
        let reused64 = stage_runtime_dll_at(&source64, uuid::Uuid::new_v4(), &local).unwrap();

        assert!(staged64.starts_with(local.join("com.aura.envbox").join("runtime")));
        assert!(staged64.ends_with("envbox-runtime64.dll"));
        assert!(staged32.ends_with("envbox-runtime32.dll"));
        assert_eq!(staged64.parent(), staged32.parent());
        assert_eq!(std::fs::read(&staged64).unwrap(), b"runtime-64");
        assert_eq!(std::fs::read(&staged32).unwrap(), b"runtime-32");
        assert_eq!(reused64, staged64);
        let _ = std::fs::remove_dir_all(dir);
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
