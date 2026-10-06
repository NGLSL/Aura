//! Read-only identity API observations. No account, adapter or registry writes.
use crate::{Field, Section, SECTION_IDENTITY};
use std::ffi::c_void;
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::Networking::WinSock::{WSACleanup, WSAStartup, WSADATA};

#[link(name = "ws2_32")]
extern "system" {
    fn gethostname(buffer: *mut u8, capacity: i32) -> i32;
    fn GetHostNameW(buffer: *mut u16, capacity: i32) -> i32;
    fn WSAGetLastError() -> i32;
}

unsafe fn winsock_names(fields: &mut Vec<Field>) {
    let mut before = [0u16; 256];
    let status = GetHostNameW(before.as_mut_ptr(), 256);
    push(
        fields,
        "GetHostNameW_BeforeStartup",
        if status == 0 {
            "initialized".to_owned()
        } else {
            format!("status={status};error={}", WSAGetLastError())
        },
    );
    let mut data = WSADATA::default();
    if WSAStartup(0x202, &mut data) != 0 {
        push(fields, "GetHostName_Startup", "failed");
        return;
    }
    for unicode in [false, true] {
        let mut a = [0u8; 256];
        let mut w = [0u16; 256];
        let api = if unicode {
            "GetHostNameW"
        } else {
            "gethostname"
        };
        let status = if unicode {
            GetHostNameW(w.as_mut_ptr(), 256)
        } else {
            gethostname(a.as_mut_ptr(), 256)
        };
        let value = if unicode {
            String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
        } else {
            String::from_utf8_lossy(&a[..a.iter().position(|&c| c == 0).unwrap_or(a.len())])
                .into_owned()
        };
        push(
            fields,
            api,
            if status == 0 {
                value.clone()
            } else {
                format!("status={status};error={}", WSAGetLastError())
            },
        );
        let short = if unicode {
            GetHostNameW(w.as_mut_ptr(), value.len() as i32)
        } else {
            gethostname(a.as_mut_ptr(), value.len() as i32)
        };
        push(
            fields,
            format!("{api}_ShortContract"),
            format!("status={short};error={}", WSAGetLastError()),
        );
        let null = if unicode {
            GetHostNameW(std::ptr::null_mut(), 256)
        } else {
            gethostname(std::ptr::null_mut(), 256)
        };
        push(
            fields,
            format!("{api}_NullContract"),
            format!("status={null};error={}", WSAGetLastError()),
        );
    }
    WSACleanup();
}

#[link(name = "kernel32")]
extern "system" {
    fn GetComputerNameA(buffer: *mut u8, size: *mut u32) -> i32;
    fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> i32;
    fn GetComputerNameExA(format: i32, buffer: *mut u8, size: *mut u32) -> i32;
    fn GetComputerNameExW(format: i32, buffer: *mut u16, size: *mut u32) -> i32;
    fn GetLastError() -> u32;
}
#[link(name = "advapi32")]
extern "system" {
    fn GetUserNameA(buffer: *mut u8, size: *mut u32) -> i32;
    fn GetUserNameW(buffer: *mut u16, size: *mut u32) -> i32;
    fn RegOpenKeyExA(
        key: isize,
        subkey: *const u8,
        options: u32,
        access: u32,
        result: *mut isize,
    ) -> i32;
    fn RegOpenKeyExW(
        key: isize,
        subkey: *const u16,
        options: u32,
        access: u32,
        result: *mut isize,
    ) -> i32;
    fn RegQueryValueExA(
        key: isize,
        name: *const u8,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegGetValueA(
        key: isize,
        subkey: *const u8,
        name: *const u8,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn RegGetValueW(
        key: isize,
        subkey: *const u16,
        name: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

fn push(fields: &mut Vec<Field>, name: impl Into<String>, value: impl Into<String>) {
    fields.push(Field {
        name: name.into(),
        value: value.into(),
    });
}
fn wide(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|&v| v == 0).unwrap_or(value.len())])
}
fn ansi(value: &[u8]) -> String {
    String::from_utf8_lossy(&value[..value.iter().position(|&v| v == 0).unwrap_or(value.len())])
        .into_owned()
}
fn mac(value: &[u8], len: u32) -> Option<String> {
    (len == 6).then(|| {
        value[..6]
            .iter()
            .map(|v| format!("{v:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    })
}
fn list(mut values: Vec<String>) -> String {
    values.sort();
    values.dedup();
    values.join(",")
}

unsafe fn names(fields: &mut Vec<Field>) {
    // Read values, successful lengths and exact-size-minus-one behavior separately.
    for user in [false, true] {
        for unicode in [false, true] {
            let name = format!(
                "{}{}",
                if user {
                    "GetUserName"
                } else {
                    "GetComputerName"
                },
                if unicode { "W" } else { "A" }
            );
            let mut a = [0xCCu8; 512];
            let mut w = [0xCCCCu16; 512];
            let mut size = 512;
            let ok = match (user, unicode) {
                (false, false) => GetComputerNameA(a.as_mut_ptr(), &mut size),
                (false, true) => GetComputerNameW(w.as_mut_ptr(), &mut size),
                (true, false) => GetUserNameA(a.as_mut_ptr(), &mut size),
                (true, true) => GetUserNameW(w.as_mut_ptr(), &mut size),
            };
            let error = GetLastError();
            push(
                fields,
                &name,
                if ok != 0 {
                    if unicode {
                        wide(&w)
                    } else {
                        ansi(&a)
                    }
                } else {
                    format!("status={error}")
                },
            );
            push(fields, format!("{name}_SuccessSize"), size.to_string());
            if ok != 0 {
                let mut short_size = if user { size.saturating_sub(1) } else { size };
                let short_ok = match (user, unicode) {
                    (false, false) => GetComputerNameA(a.as_mut_ptr(), &mut short_size),
                    (false, true) => GetComputerNameW(w.as_mut_ptr(), &mut short_size),
                    (true, false) => GetUserNameA(a.as_mut_ptr(), &mut short_size),
                    (true, true) => GetUserNameW(w.as_mut_ptr(), &mut short_size),
                };
                let short_error = GetLastError();
                push(
                    fields,
                    format!("{name}_ShortContract"),
                    format!("ok={short_ok};error={short_error};required={short_size}"),
                );
            }
        }
    }
    for format in 0..8 {
        for unicode in [false, true] {
            let name = format!(
                "GetComputerNameEx{}_{format}",
                if unicode { "W" } else { "A" }
            );
            let mut a = [0u8; 512];
            let mut w = [0u16; 512];
            let mut size = 512;
            let ok = if unicode {
                GetComputerNameExW(format, w.as_mut_ptr(), &mut size)
            } else {
                GetComputerNameExA(format, a.as_mut_ptr(), &mut size)
            };
            let error = GetLastError();
            push(
                fields,
                &name,
                if ok != 0 {
                    if unicode {
                        wide(&w)
                    } else {
                        ansi(&a)
                    }
                } else {
                    format!("status={error}")
                },
            );
            if ok != 0 {
                let mut short = size;
                let short_ok = if unicode {
                    GetComputerNameExW(format, w.as_mut_ptr(), &mut short)
                } else {
                    GetComputerNameExA(format, a.as_mut_ptr(), &mut short)
                };
                let error = GetLastError();
                push(
                    fields,
                    format!("{name}_ShortContract"),
                    format!("ok={short_ok};error={error};required={short}"),
                );
            }
        }
    }
}

unsafe fn adapters(fields: &mut Vec<Field>) {
    let mut size = 0;
    GetAdaptersAddresses(0, GET_ADAPTERS_ADDRESSES_FLAGS(0), None, None, &mut size);
    let mut storage = vec![0u64; (size as usize).div_ceil(8)];
    let first = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    let status = GetAdaptersAddresses(
        0,
        GET_ADAPTERS_ADDRESSES_FLAGS(0),
        None,
        Some(first),
        &mut size,
    );
    let mut values = Vec::new();
    if status == 0 {
        let mut next = first;
        while !next.is_null() {
            let row = &*next;
            if let Some(address) = mac(&row.PhysicalAddress, row.PhysicalAddressLength) {
                values.push(address);
            }
            next = row.Next;
        }
    }
    push(
        fields,
        "GetAdaptersAddresses_MAC",
        if status == 0 {
            list(values)
        } else {
            format!("status={status}")
        },
    );
    size = 0;
    GetAdaptersInfo(None, &mut size);
    let mut storage = vec![0u64; (size as usize).div_ceil(8)];
    let first = storage.as_mut_ptr().cast::<IP_ADAPTER_INFO>();
    let status = GetAdaptersInfo(Some(first), &mut size);
    let mut values = Vec::new();
    if status == 0 {
        let mut next = first;
        while !next.is_null() {
            let row = &*next;
            if let Some(address) = mac(&row.Address, row.AddressLength) {
                values.push(address);
            }
            next = row.Next;
        }
    }
    push(
        fields,
        "GetAdaptersInfo_MAC",
        if status == 0 {
            list(values)
        } else {
            format!("status={status}")
        },
    );
    size = 0;
    GetIfTable(None, &mut size, false);
    let mut storage = vec![0u64; (size as usize).div_ceil(8)];
    let table = storage.as_mut_ptr().cast::<MIB_IFTABLE>();
    let status = GetIfTable(Some(table), &mut size, false);
    let mut values = Vec::new();
    let mut entries = Vec::new();
    if status == 0 {
        for row in
            std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize)
        {
            if let Some(address) = mac(&row.bPhysAddr, row.dwPhysAddrLen) {
                values.push(address);
            }
            let mut entry = MIB_IFROW {
                dwIndex: row.dwIndex,
                ..Default::default()
            };
            let status = GetIfEntry(&mut entry);
            if status == 0 {
                if let Some(address) = mac(&entry.bPhysAddr, entry.dwPhysAddrLen) {
                    entries.push(address);
                }
            } else {
                entries.push(format!("status={status}"));
            }
        }
    }
    push(
        fields,
        "GetIfTable_MAC",
        if status == 0 {
            list(values)
        } else {
            format!("status={status}")
        },
    );
    push(fields, "GetIfEntry_MAC", list(entries));
    let mut table = std::ptr::null_mut();
    let status = GetIfTable2(&mut table).0;
    let mut values = Vec::new();
    let mut entries = Vec::new();
    let mut permanent = Vec::new();
    if status == 0 && !table.is_null() {
        for row in std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
        {
            if let Some(address) = mac(&row.PhysicalAddress, row.PhysicalAddressLength) {
                values.push(address);
            }
            if let Some(address) = mac(&row.PermanentPhysicalAddress, row.PhysicalAddressLength) {
                permanent.push(address);
            }
            let mut entry = MIB_IF_ROW2 {
                InterfaceIndex: row.InterfaceIndex,
                ..Default::default()
            };
            let status = GetIfEntry2(&mut entry).0;
            if status == 0 {
                if let Some(address) = mac(&entry.PhysicalAddress, entry.PhysicalAddressLength) {
                    entries.push(address);
                }
            } else {
                entries.push(format!("status={status}"));
            }
        }
        FreeMibTable(table.cast());
    }
    push(
        fields,
        "GetIfTable2_MAC",
        if status == 0 {
            list(values)
        } else {
            format!("status={status}")
        },
    );
    push(fields, "GetIfTable2_PermanentMAC", list(permanent));
    push(fields, "GetIfEntry2_MAC", list(entries));
}

struct Key(isize);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
unsafe fn machine_guid(fields: &mut Vec<Field>) {
    const HKLM: isize = -2147483646;
    let path_a = b"SOFTWARE\\Microsoft\\Cryptography\0";
    let name_a = b"MachineGuid\0";
    let path_w: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0"
        .encode_utf16()
        .collect();
    let name_w: Vec<u16> = "MachineGuid\0".encode_utf16().collect();
    for view in [0u32, 0x100, 0x200] {
        for unicode in [false, true] {
            for query in [false, true] {
                let api = format!(
                    "MachineGuid_{}{}_{view}",
                    if query {
                        "RegQueryValueEx"
                    } else {
                        "RegGetValue"
                    },
                    if unicode { "W" } else { "A" }
                );
                let mut key = HKLM;
                let owned = if query {
                    let status = if unicode {
                        RegOpenKeyExW(HKLM, path_w.as_ptr(), 0, 0x20019 | view, &mut key)
                    } else {
                        RegOpenKeyExA(HKLM, path_a.as_ptr(), 0, 0x20019 | view, &mut key)
                    };
                    if status != 0 {
                        push(fields, api, format!("open_status={status}"));
                        continue;
                    }
                    Some(Key(key))
                } else {
                    None
                };
                let mut kind = 0;
                let mut buffer = [0u16; 256];
                let mut bytes = 512;
                let mut read = |data: *mut u8, size: &mut u32| {
                    if query {
                        if unicode {
                            RegQueryValueExW(
                                key,
                                name_w.as_ptr(),
                                std::ptr::null_mut(),
                                &mut kind,
                                data,
                                size,
                            )
                        } else {
                            RegQueryValueExA(
                                key,
                                name_a.as_ptr(),
                                std::ptr::null_mut(),
                                &mut kind,
                                data,
                                size,
                            )
                        }
                    } else {
                        let flags = 2 | if view == 0x100 {
                            0x10000
                        } else if view == 0x200 {
                            0x20000
                        } else {
                            0
                        };
                        if unicode {
                            RegGetValueW(
                                key,
                                path_w.as_ptr(),
                                name_w.as_ptr(),
                                flags,
                                &mut kind,
                                data.cast(),
                                size,
                            )
                        } else {
                            RegGetValueA(
                                key,
                                path_a.as_ptr(),
                                name_a.as_ptr(),
                                flags,
                                &mut kind,
                                data.cast(),
                                size,
                            )
                        }
                    }
                };
                let status = read(buffer.as_mut_ptr().cast(), &mut bytes);
                push(
                    fields,
                    &api,
                    if status == 0 {
                        if unicode {
                            wide(&buffer)
                        } else {
                            ansi(std::slice::from_raw_parts(
                                buffer.as_ptr().cast(),
                                bytes as usize,
                            ))
                        }
                    } else {
                        format!("status={status}")
                    },
                );
                let mut required = 0;
                let size_status = read(std::ptr::null_mut(), &mut required);
                let mut short = required.saturating_sub(1);
                let short_status = read(buffer.as_mut_ptr().cast(), &mut short);
                drop(read);
                push(fields, format!("{api}_Contract"), format!("type={kind};size_status={size_status};required={required};short_status={short_status};short_required={short}"));
                if !query {
                    // Larger backing storage detects writes beyond the advertised
                    // two-byte capacity without turning an API bug into Probe UB.
                    let mut guard = [0xABABu16; 128];
                    let mut capacity = 2;
                    let flags = 2
                        | 0x20000000
                        | if view == 0x100 {
                            0x10000
                        } else if view == 0x200 {
                            0x20000
                        } else {
                            0
                        };
                    let status = if unicode {
                        RegGetValueW(
                            key,
                            path_w.as_ptr(),
                            name_w.as_ptr(),
                            flags,
                            &mut kind,
                            guard.as_mut_ptr().cast(),
                            &mut capacity,
                        )
                    } else {
                        RegGetValueA(
                            key,
                            path_a.as_ptr(),
                            name_a.as_ptr(),
                            flags,
                            &mut kind,
                            guard.as_mut_ptr().cast(),
                            &mut capacity,
                        )
                    };
                    let guard_bytes = std::slice::from_raw_parts(guard.as_ptr().cast::<u8>(), 256);
                    push(
                        fields,
                        format!("{api}_ZeroOnFailure"),
                        format!(
                            "status={status};required={capacity};zeroed={};canary={}",
                            guard_bytes[..2].iter().all(|&v| v == 0),
                            guard_bytes[2..].iter().all(|&v| v == 0xAB)
                        ),
                    );
                }
                drop(owned);
            }
        }
    }
}

pub fn collect() -> Section {
    let mut fields = Vec::new();
    unsafe {
        names(&mut fields);
        winsock_names(&mut fields);
        adapters(&mut fields);
        machine_guid(&mut fields);
    }
    for name in ["COMPUTERNAME", "USERNAME"] {
        push(
            &mut fields,
            format!("Environment_{name}"),
            std::env::var(name).unwrap_or_default(),
        );
    }
    Section {
        title: SECTION_IDENTITY.into(),
        fields,
    }
}
