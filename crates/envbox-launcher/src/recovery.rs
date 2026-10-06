//! Reconfirm an already loaded Runtime. This never injects a DLL or claims a PID.
use std::{path::Path, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("Runtime recovery refused: {0}")]
    Refused(String),
    #[error("Runtime recovery timed out; the original process remains alive")]
    Timeout,
}

fn refused(value: impl ToString) -> RecoveryError {
    RecoveryError::Refused(value.to_string())
}

fn read_image(path: &Path) -> Result<Vec<u8>, RecoveryError> {
    use std::io::Read;
    const MAX_IMAGE: u64 = 64 * 1024 * 1024;
    let file = std::fs::File::open(path).map_err(refused)?;
    let metadata = file.metadata().map_err(refused)?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE {
        return Err(refused("Runtime PE exceeds bounded image size"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE + 1)
        .read_to_end(&mut bytes)
        .map_err(refused)?;
    if bytes.len() as u64 > MAX_IMAGE {
        return Err(refused("Runtime PE grew beyond bounded image size"));
    }
    Ok(bytes)
}

// Resolve only the dedicated, non-forwarded export from a bounded file image.
// All RVA reads must map to file-backed section bytes; the entry must be executable.
fn export_rva(
    bytes: &[u8],
    export_name: &[u8],
    executable: bool,
) -> Result<(u32, u32, u16, usize), RecoveryError> {
    let u16at = |at: usize| {
        bytes
            .get(at..at.checked_add(2).ok_or_else(|| refused("PE overflow"))?)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| refused("truncated PE"))
    };
    let u32at = |at: usize| {
        bytes
            .get(at..at.checked_add(4).ok_or_else(|| refused("PE overflow"))?)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| refused("truncated PE"))
    };
    if bytes.get(..2) != Some(b"MZ") {
        return Err(refused("not PE"));
    }
    let pe = u32at(0x3c)? as usize;
    if bytes.get(pe..pe.checked_add(4).ok_or_else(|| refused("PE overflow"))?) != Some(b"PE\0\0") {
        return Err(refused("bad PE signature"));
    }
    let machine = u16at(pe + 4)?;
    let sections = u16at(pe + 6)? as usize;
    let optional_size = u16at(pe + 20)? as usize;
    let optional = pe.checked_add(24).ok_or_else(|| refused("PE overflow"))?;
    let directory = match (machine, u16at(optional)?) {
        (0x14c, 0x10b) => 96,
        (0x8664, 0x20b) => 112,
        _ => return Err(refused("unsupported PE architecture")),
    };
    if optional_size < directory + 8 || sections == 0 || sections > 96 {
        return Err(refused("bad PE headers"));
    }
    let image_size = u32at(optional + 56)?;
    let exports = u32at(optional + directory)?;
    let export_size = u32at(optional + directory + 4)?;
    let section_start = optional
        .checked_add(optional_size)
        .ok_or_else(|| refused("PE overflow"))?;
    let map = |rva: u32, length: usize, executable: bool| -> Result<usize, RecoveryError> {
        for i in 0..sections {
            let at = section_start
                .checked_add(i * 40)
                .ok_or_else(|| refused("PE overflow"))?;
            let va = u32at(at + 12)?;
            let size = u32at(at + 16)?;
            let raw = u32at(at + 20)?;
            if rva >= va && (rva - va) as u64 + length as u64 <= size as u64 {
                if executable && u32at(at + 36)? & 0x20000000 == 0 {
                    return Err(refused("non-executable export"));
                }
                let offset = (raw as usize)
                    .checked_add((rva - va) as usize)
                    .ok_or_else(|| refused("PE overflow"))?;
                bytes
                    .get(
                        offset
                            ..offset
                                .checked_add(length)
                                .ok_or_else(|| refused("PE overflow"))?,
                    )
                    .ok_or_else(|| refused("bad PE section"))?;
                return Ok(offset);
            }
        }
        Err(refused("unmapped PE RVA"))
    };
    if exports == 0 || export_size < 40 {
        return Err(refused("old Runtime has no reconnect export"));
    }
    let table = map(exports, 40, false)?;
    let functions = u32at(table + 20)?;
    let names = u32at(table + 24)?;
    if functions > 65536 || names > 65536 {
        return Err(refused("oversized export table"));
    }
    let function_table = map(u32at(table + 28)?, functions as usize * 4, false)?;
    let name_table = map(u32at(table + 32)?, names as usize * 4, false)?;
    let ordinals = map(u32at(table + 36)?, names as usize * 2, false)?;
    for i in 0..names as usize {
        let name_rva = u32at(name_table + i * 4)?;
        let expected = export_name;
        let name_offset = map(name_rva, expected.len(), false)?;
        if &bytes[name_offset..name_offset + expected.len()] != expected {
            continue;
        }
        let ordinal = u16at(ordinals + i * 2)? as u32;
        if ordinal >= functions {
            return Err(refused("bad export ordinal"));
        }
        let rva = u32at(function_table + ordinal as usize * 4)?;
        if rva >= exports && (rva as u64) < exports as u64 + export_size as u64 {
            return Err(refused("forwarded reconnect export"));
        }
        if rva == 0 || rva >= image_size {
            return Err(refused("bad reconnect RVA"));
        }
        let file_offset = map(rva, if executable { 1 } else { 256 }, executable)?;
        return Ok((rva, image_size, machine, file_offset));
    }
    Err(refused("old Runtime has no reconnect export"))
}

/// The caller first rebuilds the immutable Profile registry and starts its
/// instance broker. Success requires a new authenticated challenge handshake;
/// the caller must additionally check SessionTable::runtime_reconfirmed(pid).
/// Timeout never kills the thread/process; NULL is its only parameter.
#[cfg(windows)]
pub fn request_runtime_reconnect(
    pid: u32,
    generation: u64,
    module_path: &Path,
    module_sha256: &str,
    timeout: Duration,
) -> Result<(), RecoveryError> {
    use crate::launcher::win::SafeHandle;
    use sha2::{Digest, Sha256};
    use windows::Win32::{
        Foundation::{FILETIME, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W,
                TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
            },
            Threading::{
                CreateRemoteThread, GetExitCodeThread, GetProcessTimes, OpenProcess,
                WaitForSingleObject, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION,
                PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
            },
        },
    };
    let started = std::time::Instant::now();
    if timeout.is_zero() || timeout > Duration::from_secs(5) {
        return Err(refused("invalid recovery deadline"));
    }
    let path = std::fs::canonicalize(module_path).map_err(refused)?;
    let bytes = read_image(&path)?;
    if format!("{:x}", Sha256::digest(&bytes)) != module_sha256 {
        return Err(refused("sealed Runtime hash mismatch"));
    }
    let (rva, image_size, machine, _) = export_rva(&bytes, b"EnvBoxRuntimeReconnect\0", true)?;
    if usize::BITS == 32 && machine == 0x8664 {
        return Err(refused("32-bit Host cannot reconnect 64-bit Runtime"));
    }
    unsafe {
        let process = SafeHandle(
            OpenProcess(
                PROCESS_CREATE_THREAD
                    | PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_OPERATION
                    | PROCESS_VM_READ
                    | PROCESS_VM_WRITE,
                false,
                pid,
            )
            .map_err(refused)?,
        );
        let check_generation = || -> Result<(), RecoveryError> {
            let mut created = FILETIME::default();
            let mut exited = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
                .map_err(refused)?;
            if ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64 != generation
            {
                return Err(refused("PID generation mismatch"));
            }
            Ok(())
        };
        check_generation()?;
        let snapshot = SafeHandle(
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)
                .map_err(refused)?,
        );
        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        Module32FirstW(snapshot.0, &mut entry).map_err(refused)?;
        let base = loop {
            let end = entry
                .szExePath
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExePath.len());
            if std::fs::canonicalize(String::from_utf16_lossy(&entry.szExePath[..end]))
                .ok()
                .as_ref()
                == Some(&path)
            {
                if entry.modBaseSize != image_size {
                    return Err(refused("loaded Runtime image size mismatch"));
                }
                break entry.modBaseAddr as usize;
            }
            Module32NextW(snapshot.0, &mut entry)
                .map_err(|_| refused("sealed Runtime is not loaded"))?;
        };
        check_generation()?;
        if crate::ipc_server::file_sha256(&path).map_err(refused)? != module_sha256 {
            return Err(refused("Runtime changed before reconnect"));
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or(RecoveryError::Timeout)?;
        let address = base
            .checked_add(rva as usize)
            .ok_or_else(|| refused("remote export overflow"))?;
        let thread = SafeHandle(
            CreateRemoteThread(
                process.0,
                None,
                0,
                Some(std::mem::transmute::<
                    usize,
                    unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
                >(address)),
                None,
                0,
                None,
            )
            .map_err(refused)?,
        );
        let wait =
            WaitForSingleObject(thread.0, remaining.as_millis().min(u32::MAX as u128) as u32);
        if wait == WAIT_TIMEOUT {
            return Err(RecoveryError::Timeout);
        }
        if wait != WAIT_OBJECT_0 {
            return Err(refused("reconnect thread wait failed"));
        }
        let mut exit = 1;
        GetExitCodeThread(thread.0, &mut exit).map_err(refused)?;
        if exit != 0 {
            return Err(refused(format!("Runtime reconnect rejected ({exit})")));
        }
        check_generation()?;
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn request_runtime_reconnect(
    _: u32,
    _: u64,
    _: &Path,
    _: &str,
    _: Duration,
) -> Result<(), RecoveryError> {
    Err(refused("Windows only"))
}

/// Offline, bounded DATA-export inspection. No DLL code is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    pub protocol: u32,
    pub profile_dns_schema: u32,
    pub entry_gate: bool,
    pub reconnect: bool,
    pub dns_udp: bool,
    pub dns_tcp: bool,
    pub dns_dot: bool,
    pub dns_doh: bool,
}

/// Preflight the exact staged DLL before activation/injection. UDP profiles
/// require TCP as well, because truncated DNS replies retry over TCP.
pub fn validate_runtime_for_profile(
    path: &Path,
    dns: &envbox_core::DnsProfile,
    entry_gate: bool,
) -> Result<(), RecoveryError> {
    use envbox_core::{DnsMode, DnsUpstream};
    if !entry_gate && dns.mode != DnsMode::VirtualView {
        return Ok(());
    }
    let capabilities = read_runtime_capabilities(path)?;
    if entry_gate && !capabilities.entry_gate {
        return Err(refused("Runtime lacks entry gate capability"));
    }
    if dns.mode == DnsMode::VirtualView {
        for upstream in dns.effective_upstreams() {
            let supported = match upstream {
                DnsUpstream::Udp { .. } => capabilities.dns_udp && capabilities.dns_tcp,
                DnsUpstream::Tcp { .. } => capabilities.dns_tcp,
                DnsUpstream::Dot { .. } => capabilities.dns_dot,
                DnsUpstream::Doh { .. } => capabilities.dns_doh,
            };
            if !supported {
                return Err(refused(
                    "staged Runtime lacks requested DNS transport capability",
                ));
            }
        }
    }
    Ok(())
}

pub fn read_runtime_capabilities(path: &Path) -> Result<RuntimeCapabilities, RecoveryError> {
    let bytes = read_image(path)?;
    let (_, _, _, offset) = export_rva(&bytes, b"EnvBoxRuntimeCapabilities\0", false)?;
    let data = &bytes[offset..offset + 256];
    let end = data
        .iter()
        .position(|c| *c == 0)
        .ok_or_else(|| refused("unterminated capability data"))?;
    if !data[..end].is_ascii() {
        return Err(refused("non-ASCII capability data"));
    }
    let text = std::str::from_utf8(&data[..end]).map_err(refused)?;
    let keys = [
        "protocol",
        "profile_dns_schema",
        "entry_gate",
        "reconnect",
        "dns_udp",
        "dns_tcp",
        "dns_dot",
        "dns_doh",
    ];
    let mut fields = std::collections::HashMap::new();
    for item in text.split(';') {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| refused("bad capability field"))?;
        if !keys.contains(&key)
            || fields.insert(key, value).is_some()
            || !matches!(value, "0" | "1")
        {
            return Err(refused("unknown/duplicate capability field"));
        }
    }
    if fields.len() != keys.len()
        || fields["protocol"] != "1"
        || fields["profile_dns_schema"] != "1"
    {
        return Err(refused("unsupported Runtime capability schema"));
    }
    Ok(RuntimeCapabilities {
        protocol: 1,
        profile_dns_schema: 1,
        entry_gate: fields["entry_gate"] == "1",
        reconnect: fields["reconnect"] == "1",
        dns_udp: fields["dns_udp"] == "1",
        dns_tcp: fields["dns_tcp"] == "1",
        dns_dot: fields["dns_dot"] == "1",
        dns_doh: fields["dns_doh"] == "1",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x600];
        fn u16put(b: &mut [u8], at: usize, v: u16) {
            b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        }
        fn u32put(b: &mut [u8], at: usize, v: u32) {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[..2].copy_from_slice(b"MZ");
        u32put(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        u16put(&mut bytes, 0x84, 0x8664);
        u16put(&mut bytes, 0x86, 1);
        u16put(&mut bytes, 0x94, 0xf0);
        u16put(&mut bytes, 0x98, 0x20b);
        u32put(&mut bytes, 0x98 + 56, 0x2000);
        u32put(&mut bytes, 0x98 + 112, 0x1000);
        u32put(&mut bytes, 0x98 + 116, 0x100);
        let section = 0x188;
        u32put(&mut bytes, section + 12, 0x1000);
        u32put(&mut bytes, section + 16, 0x400);
        u32put(&mut bytes, section + 20, 0x200);
        u32put(&mut bytes, section + 36, 0x40000040);
        u32put(&mut bytes, 0x200 + 20, 1);
        u32put(&mut bytes, 0x200 + 24, 1);
        u32put(&mut bytes, 0x200 + 28, 0x1040);
        u32put(&mut bytes, 0x200 + 32, 0x1044);
        u32put(&mut bytes, 0x200 + 36, 0x1048);
        u32put(&mut bytes, 0x240, 0x1100);
        u32put(&mut bytes, 0x244, 0x1080);
        let name = b"EnvBoxRuntimeCapabilities\0";
        bytes[0x280..0x280 + name.len()].copy_from_slice(name);
        let data=b"protocol=1;profile_dns_schema=1;entry_gate=1;reconnect=1;dns_udp=1;dns_tcp=1;dns_dot=0;dns_doh=0\0";
        bytes[0x300..0x300 + data.len()].copy_from_slice(data);
        bytes
    }
    #[test]
    fn pe_exports_reject_forwarders_bad_sections_and_executable_data_confusion() {
        let bytes = image();
        assert!(export_rva(&bytes, b"EnvBoxRuntimeCapabilities\0", false).is_ok());
        assert!(export_rva(&bytes, b"EnvBoxRuntimeCapabilities\0", true).is_err());
        let mut forwarded = bytes.clone();
        forwarded[0x240..0x244].copy_from_slice(&0x1080u32.to_le_bytes());
        assert!(export_rva(&forwarded, b"EnvBoxRuntimeCapabilities\0", false).is_err());
        assert!(export_rva(&bytes[..0x320], b"EnvBoxRuntimeCapabilities\0", false).is_err());
        assert!(export_rva(&bytes, b"EnvBoxRuntimeReconnect\0", true).is_err());
    }
    #[test]
    fn offline_capabilities_reject_unknown_and_unterminated_data() {
        let path = std::env::temp_dir().join(format!(
            "aura-capability-fixture-{}.dll",
            uuid::Uuid::new_v4()
        ));
        let mut bytes = image();
        std::fs::write(&path, &bytes).unwrap();
        let caps = read_runtime_capabilities(&path).unwrap();
        assert!(caps.entry_gate && caps.reconnect);
        assert!(!caps.dns_doh);
        let virtual_dns = envbox_core::DnsProfile::from_servers(
            envbox_core::DnsMode::VirtualView,
            vec!["1.1.1.1".parse().unwrap()],
        );
        assert!(validate_runtime_for_profile(&path, &virtual_dns, true).is_ok());
        let unavailable = envbox_core::DnsProfile::typed(
            envbox_core::DnsMode::VirtualView,
            true,
            vec![envbox_core::DnsUpstream::Doh {
                url: "https://resolver.example.test/dns-query".into(),
                bootstrap_ips: vec!["1.1.1.1".parse().unwrap()],
                tls_revocation: envbox_core::DnsTlsRevocation::Standard,
            }],
        );
        unavailable.validate_runtime_support().unwrap();
        assert!(validate_runtime_for_profile(&path, &unavailable, true).is_err());
        assert!(validate_runtime_for_profile(
            Path::new("nonexistent-runtime.dll"),
            &virtual_dns,
            true
        )
        .is_err());
        assert!(validate_runtime_for_profile(
            Path::new("nonexistent-runtime.dll"),
            &envbox_core::DnsProfile::default(),
            false
        )
        .is_ok());
        bytes[0x300] = b'x';
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_runtime_capabilities(&path).is_err());
        bytes[0x300..0x400].fill(b'a');
        std::fs::write(&path, &bytes).unwrap();
        assert!(read_runtime_capabilities(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
