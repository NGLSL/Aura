//! Arbitrary record queries for the injected Windows DNS acceptance fixture.
use super::{DnsQueryCancel, DnsQueryEx, DnsQueryRequest, DnsQueryResult};
use std::ffi::{c_void, CStr, CString};
use std::process::ExitCode;
use std::sync::mpsc;

#[repr(C)]
struct Record {
    next: *mut Record,
    name: *mut c_void,
    kind: u16,
    length: u16,
    flags: u32,
    ttl: u32,
    reserved: u32,
    data: [u8; 0],
}

#[link(name = "dnsapi")]
extern "system" {
    fn DnsQuery_A(
        name: *const u8,
        kind: u16,
        options: u32,
        servers: *mut c_void,
        records: *mut *mut c_void,
        reserved: *mut *mut c_void,
    ) -> i32;
    fn DnsQuery_UTF8(
        name: *const u8,
        kind: u16,
        options: u32,
        servers: *mut c_void,
        records: *mut *mut Record,
        reserved: *mut c_void,
    ) -> i32;
    fn DnsQuery_W(
        name: *const u16,
        kind: u16,
        options: u32,
        servers: *mut c_void,
        records: *mut *mut Record,
        reserved: *mut c_void,
    ) -> i32;
    fn DnsRecordListFree(records: *mut Record, free_type: u32);
}

unsafe fn string(ptr: *const c_void, wide: bool) -> String {
    if ptr.is_null() {
        return String::new();
    }
    if wide {
        let ptr = ptr.cast::<u16>();
        let mut count = 0;
        while count < 4096 && *ptr.add(count) != 0 {
            count += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, count))
    } else {
        CStr::from_ptr(ptr.cast()).to_string_lossy().into_owned()
    }
}

unsafe fn describe(mut record: *mut Record, wide: bool) -> Vec<String> {
    let mut rows = Vec::new();
    while !record.is_null() && rows.len() < 128 {
        let r = &*record;
        let data = r.data.as_ptr();
        let value = match r.kind {
            1 => std::slice::from_raw_parts(data, 4)
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join("."),
            5 | 12 | 2 => string(*(data.cast::<*const c_void>()), wide),
            16 => {
                let count = *(data.cast::<u32>()) as usize;
                let pointers = data
                    .add(if cfg!(target_pointer_width = "64") {
                        8
                    } else {
                        4
                    })
                    .cast::<*const c_void>();
                (0..count.min(128))
                    .map(|i| string(*pointers.add(i), wide))
                    .collect::<Vec<_>>()
                    .join("|")
            }
            33 => format!(
                "{}:{}:{}:{}",
                string(*(data.cast::<*const c_void>()), wide),
                *data.add(std::mem::size_of::<usize>()).cast::<u16>(),
                *data.add(std::mem::size_of::<usize>() + 2).cast::<u16>(),
                *data.add(std::mem::size_of::<usize>() + 4).cast::<u16>()
            ),
            64 | 65 | 65280 => std::slice::from_raw_parts(data, r.length as usize)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            _ => "opaque".into(),
        };
        rows.push(format!(
            "type={} length={} name={} value={}",
            r.kind,
            r.length,
            string(r.name, wide),
            value
        ));
        record = r.next;
    }
    rows
}

unsafe extern "system" fn completion(context: *mut c_void, result: *mut DnsQueryResult) {
    let sender = (&*context.cast::<mpsc::Sender<(i32, Vec<String>)>>()).clone();
    let rows = describe((*result).query_records.cast(), true);
    DnsRecordListFree((*result).query_records.cast(), 1);
    (*result).query_records = std::ptr::null_mut();
    let status = (*result).query_status;
    let _ = sender.send((status, rows));
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(kind) = args.get(1).and_then(|s| s.parse::<u16>().ok()) else {
        eprintln!("usage: --dns-rr NAME QTYPE a|w|utf8|ex|async");
        return ExitCode::FAILURE;
    };
    let Some(api) = args
        .get(2)
        .filter(|s| matches!(s.as_str(), "a" | "w" | "utf8" | "ex" | "async"))
    else {
        eprintln!("usage: --dns-rr NAME QTYPE a|w|utf8|ex|async");
        return ExitCode::FAILURE;
    };
    let Ok(name) = CString::new(args[0].as_str()) else {
        return ExitCode::FAILURE;
    };
    let wide: Vec<u16> = args[0].encode_utf16().chain(Some(0)).collect();
    let options = args
        .get(3)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0x108);
    super::print_runtime_marker();
    unsafe {
        let mut records = std::ptr::null_mut();
        let (status, rows) = if api == "ex" || api == "async" {
            let (sender, receiver) = mpsc::channel();
            // Keep result, request and callback context alive through completion.
            let sender = Box::into_raw(Box::new(sender));
            let result = Box::into_raw(Box::new(DnsQueryResult {
                version: 1,
                query_status: 0,
                query_options: 0,
                query_records: std::ptr::null_mut(),
                reserved: std::ptr::null_mut(),
            }));
            let request = DnsQueryRequest {
                version: 1,
                query_name: wide.as_ptr(),
                query_type: kind,
                query_options: options as u64,
                dns_server_list: std::ptr::null_mut(),
                interface_index: 0,
                completion: if api == "async" {
                    Some(completion)
                } else {
                    None
                },
                query_context: sender.cast(),
            };
            let mut cancel = DnsQueryCancel { reserved: [0; 32] };
            let returned = DnsQueryEx(
                &request,
                result,
                if api == "async" {
                    &mut cancel
                } else {
                    std::ptr::null_mut()
                },
            );
            println!("DnsRR_ReturnStatus:\n{returned}");
            if returned == 9506 && args.iter().any(|arg| arg == "--cancel") {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let cancelled = super::DnsCancelQuery(&cancel);
                println!("DnsRR_CancelStatus:\n{cancelled}");
            }
            let outcome = if returned == 9506 {
                match receiver.recv_timeout(std::time::Duration::from_secs(15)) {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        eprintln!("DNS completion timed out");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                let rows = describe((*result).query_records.cast(), true);
                DnsRecordListFree((*result).query_records.cast(), 1);
                (
                    if returned == 0 {
                        (*result).query_status
                    } else {
                        returned
                    },
                    rows,
                )
            };
            drop(Box::from_raw(sender));
            drop(Box::from_raw(result));
            outcome
        } else {
            let status = match api.as_str() {
                "a" => DnsQuery_A(
                    name.as_ptr().cast(),
                    kind,
                    options,
                    std::ptr::null_mut(),
                    (&mut records as *mut *mut Record).cast(),
                    std::ptr::null_mut(),
                ),
                "utf8" => DnsQuery_UTF8(
                    name.as_ptr().cast(),
                    kind,
                    options,
                    std::ptr::null_mut(),
                    &mut records,
                    std::ptr::null_mut(),
                ),
                _ => DnsQuery_W(
                    wide.as_ptr(),
                    kind,
                    options,
                    std::ptr::null_mut(),
                    &mut records,
                    std::ptr::null_mut(),
                ),
            };
            let rows = describe(records, api == "w");
            DnsRecordListFree(records, 1);
            (status, rows)
        };
        println!("DnsRR_Status:\n{status}\nDnsRR_Records:\n{}", rows.len());
        for row in rows {
            println!("DnsRR_Record: {row}");
        }
        println!("DnsRR_Freed:\ntrue");
    }
    ExitCode::SUCCESS
}
