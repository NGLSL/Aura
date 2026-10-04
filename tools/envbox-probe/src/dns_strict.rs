//! Native unsupported-entrypoint probes. Each command runs in its own process.
use super::{DnsQueryCancel, DnsQueryEx, DnsQueryRequest, DnsQueryResult};
use std::ffi::{c_void, CString};
use std::process::ExitCode;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use windows::core::{GUID, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Networking::WinSock::{
    FreeAddrInfoEx, FreeAddrInfoExW, GetAddrInfoExA, GetAddrInfoExCancel,
    GetAddrInfoExOverlappedResult, GetAddrInfoExW, WSAGetLastError, WSAStartup, ADDRINFOEXA,
    ADDRINFOEXW, TIMEVAL, WSADATA,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::IO::OVERLAPPED;

#[repr(C)]
struct ExState {
    overlapped: OVERLAPPED,
    done: HANDLE,
    callbacks: AtomicU32,
    callback_status: AtomicI32,
    result_a: *mut ADDRINFOEXA,
    result_w: *mut ADDRINFOEXW,
    token: HANDLE,
}

unsafe extern "system" fn ex_completion(error: u32, _: u32, overlapped: *const OVERLAPPED) {
    let state = &*(overlapped.cast::<ExState>());
    state.callback_status.store(error as i32, Ordering::Release);
    state.callbacks.fetch_add(1, Ordering::AcqRel);
    let _ = SetEvent(state.done);
}

unsafe fn ex(api: &str, mode: &str) -> ExitCode {
    let mut wsa = WSADATA::default();
    let startup = WSAStartup(0x202, &mut wsa);
    if startup != 0 {
        println!("StrictProbe_Startup:\n{startup}");
        return ExitCode::FAILURE;
    }
    let done = CreateEventW(None, true, false, None).expect("completion event");
    let mut state = Box::new(ExState {
        overlapped: OVERLAPPED::default(),
        done,
        callbacks: AtomicU32::new(0),
        callback_status: AtomicI32::new(-1),
        result_a: std::ptr::null_mut(),
        result_w: std::ptr::null_mut(),
        token: HANDLE::default(),
    });
    let event_mode = mode == "event";
    let asynchronous = event_mode || mode == "callback";
    if event_mode {
        state.overlapped.hEvent = done;
    }
    let name = CString::new("strict.fixture.test").unwrap();
    let wide: Vec<u16> = "strict.fixture.test"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let namespace = if mode == "namespace" { 37 } else { 12 };
    let provider = GUID::from_u128(0x11223344_5566_7788_99aa_bbccddeeff00);
    let provider_ptr = if mode == "provider" {
        Some(&provider as *const GUID)
    } else {
        None
    };
    let timeout = TIMEVAL {
        tv_sec: if mode == "deadline" { 1 } else { 3 },
        tv_usec: 0,
    };
    let timeout_ptr = if api == "w" && !asynchronous {
        Some(&timeout as *const TIMEVAL)
    } else {
        None
    };
    let token_ptr = if asynchronous {
        Some(&mut state.token as *mut HANDLE)
    } else {
        None
    };
    let overlapped = if asynchronous {
        Some(&state.overlapped as *const OVERLAPPED)
    } else {
        None
    };
    let completion = if mode == "callback" {
        Some(ex_completion as unsafe extern "system" fn(u32, u32, *const OVERLAPPED))
    } else {
        None
    };
    let status = if api == "a" {
        GetAddrInfoExA(
            PCSTR(name.as_ptr().cast()),
            PCSTR::null(),
            namespace,
            provider_ptr,
            None,
            &mut state.result_a,
            timeout_ptr,
            overlapped,
            completion,
            token_ptr,
        )
    } else {
        GetAddrInfoExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            namespace,
            provider_ptr,
            None,
            &mut state.result_w,
            timeout_ptr,
            overlapped,
            completion,
            token_ptr,
        )
    };
    let last_error = WSAGetLastError().0;
    let initial_callbacks = state.callbacks.load(Ordering::Acquire);
    let initial_event = WaitForSingleObject(done, 0).0;
    let initial_token = !state.token.is_invalid() && !state.token.0.is_null();
    println!("StrictProbe_Status:\n{status}\nStrictProbe_LastError:\n{last_error}\nStrictProbe_InitialCallbacks:\n{initial_callbacks}\nStrictProbe_InitialEvent:\n{initial_event}\nStrictProbe_InitialToken:\n{initial_token}");
    let pending = status == 997;
    if pending {
        let cancelled = GetAddrInfoExCancel(&state.token);
        let wait = WaitForSingleObject(done, 5000);
        println!(
            "StrictProbe_Cancel:\n{cancelled}\nStrictProbe_DrainEvent:\n{}",
            wait.0
        );
        if wait == WAIT_OBJECT_0 {
            println!(
                "StrictProbe_DrainStatus:\n{}",
                GetAddrInfoExOverlappedResult(&state.overlapped)
            );
        }
    } else {
        // Observe whether a supposedly synchronous rejection later produces a callback.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    println!(
        "StrictProbe_FinalCallbacks:\n{}\nStrictProbe_FinalEvent:\n{}\nStrictProbe_Records:\n{}",
        state.callbacks.load(Ordering::Acquire),
        WaitForSingleObject(done, 0).0,
        u32::from(!state.result_a.is_null() || !state.result_w.is_null())
    );
    if pending {
        println!("StrictProbe_Lifetime:\nretained_until_process_exit");
        // Native providers can outlive cancellation. Do not drop name, timeout,
        // OVERLAPPED, token or result slots while an operation can still borrow them.
        std::process::exit(0);
    }
    if !state.result_a.is_null() {
        FreeAddrInfoEx(Some(state.result_a));
    }
    if !state.result_w.is_null() {
        FreeAddrInfoExW(Some(state.result_w));
    }
    let _ = CloseHandle(done);
    ExitCode::SUCCESS
}

#[repr(C)]
struct RawRequest {
    version: u32,
    results_version: u32,
    packet_size: u32,
    packet: *mut u8,
    name: *mut u16,
    kind: u16,
    options: u64,
    interface: u32,
    completion: Option<unsafe extern "system" fn(*mut c_void, *mut c_void)>,
    context: *mut c_void,
    raw_options: u64,
    servers_size: u32,
    servers: *mut c_void,
    protocol: u32,
    source: [u8; 32],
}

#[repr(C)]
struct RawState {
    done: HANDLE,
    callbacks: AtomicU32,
    callback_status: AtomicI32,
    free: unsafe extern "system" fn(*mut c_void),
}

unsafe extern "system" fn raw_completion(context: *mut c_void, result: *mut c_void) {
    let state = &*(context.cast::<RawState>());
    let status = if result.is_null() {
        -1
    } else {
        *result.cast::<i32>().add(1)
    };
    (state.free)(result);
    state.callback_status.store(status, Ordering::Release);
    state.callbacks.fetch_add(1, Ordering::AcqRel);
    let _ = SetEvent(state.done);
}

unsafe fn raw(mode: &str) -> ExitCode {
    let module = GetModuleHandleW(windows::core::w!("dnsapi.dll")).expect("dnsapi loaded");
    let query = GetProcAddress(module, windows::core::s!("DnsQueryRaw"));
    let cancel = GetProcAddress(module, windows::core::s!("DnsCancelQueryRaw"));
    let free = GetProcAddress(module, windows::core::s!("DnsQueryRawResultFree"));
    let (Some(query), Some(cancel), Some(free)) = (query, cancel, free) else {
        println!("StrictProbe_Available:\nfalse");
        return ExitCode::SUCCESS;
    };
    println!(
        "StrictProbe_Available:\ntrue\nStrictProbe_RawRequestSize:\n{}",
        std::mem::size_of::<RawRequest>()
    );
    let query: unsafe extern "system" fn(*mut RawRequest, *mut DnsQueryCancel) -> i32 =
        std::mem::transmute(query);
    let cancel: unsafe extern "system" fn(*mut DnsQueryCancel) -> i32 = std::mem::transmute(cancel);
    let free = std::mem::transmute(free);
    let done = CreateEventW(None, true, false, None).expect("raw completion event");
    let mut state = Box::new(RawState {
        done,
        callbacks: AtomicU32::new(0),
        callback_status: AtomicI32::new(-1),
        free,
    });
    let mut name: Vec<u16> = "strict.fixture.test"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut packet = vec![
        0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 6, b's', b't', b'r', b'i', b'c', b't', 7, b'f',
        b'i', b'x', b't', b'u', b'r', b'e', 4, b't', b'e', b's', b't', 0, 0, 1, 0, 1,
    ];
    let mut token = DnsQueryCancel { reserved: [0; 32] };
    let mut request = RawRequest {
        version: 1,
        results_version: 1,
        packet_size: if mode == "packet" {
            packet.len() as u32
        } else {
            0
        },
        packet: if mode == "packet" {
            packet.as_mut_ptr()
        } else {
            std::ptr::null_mut()
        },
        name: if mode == "name" {
            name.as_mut_ptr()
        } else {
            std::ptr::null_mut()
        },
        kind: if mode == "packet" { 0 } else { 1 },
        options: 0x108,
        interface: 0,
        completion: Some(raw_completion),
        context: (&mut *state as *mut RawState).cast(),
        raw_options: 0,
        servers_size: 0,
        servers: std::ptr::null_mut(),
        protocol: 1,
        source: {
            let mut address = [0; 32];
            address[0] = 2;
            address[4..8].copy_from_slice(&[127, 0, 0, 1]);
            address
        },
    };
    let status = query(&mut request, &mut token);
    let initial_callbacks = state.callbacks.load(Ordering::Acquire);
    let initial_event = WaitForSingleObject(done, 0).0;
    let token_changed = token.reserved.iter().any(|byte| *byte != 0);
    println!("StrictProbe_Status:\n{status}\nStrictProbe_InitialCallbacks:\n{initial_callbacks}\nStrictProbe_InitialEvent:\n{initial_event}\nStrictProbe_InitialToken:\n{token_changed}");
    let pending = status == 9506;
    if pending {
        let cancelled = cancel(&mut token);
        let wait = WaitForSingleObject(done, 5000).0;
        println!("StrictProbe_Cancel:\n{cancelled}\nStrictProbe_DrainEvent:\n{wait}");
    } else {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    println!(
        "StrictProbe_FinalCallbacks:\n{}\nStrictProbe_FinalEvent:\n{}",
        state.callbacks.load(Ordering::Acquire),
        WaitForSingleObject(done, 0).0
    );
    if pending {
        println!("StrictProbe_Lifetime:\nretained_until_process_exit");
        std::process::exit(0);
    }
    let _ = CloseHandle(done);
    ExitCode::SUCCESS
}

unsafe fn null_ex(mode: &str) -> ExitCode {
    let name: Vec<u16> = "strict.fixture.test"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let request = DnsQueryRequest {
        version: 1,
        query_name: name.as_ptr(),
        query_type: 65,
        query_options: 0x108,
        dns_server_list: std::ptr::null_mut(),
        interface_index: 0,
        completion: None,
        query_context: std::ptr::null_mut(),
    };
    let mut result = DnsQueryResult {
        version: 1,
        query_status: 0,
        query_options: 0,
        query_records: std::ptr::null_mut(),
        reserved: std::ptr::null_mut(),
    };
    let status = DnsQueryEx(
        if mode == "request" {
            std::ptr::null()
        } else {
            &request
        },
        if mode == "result" {
            std::ptr::null_mut()
        } else {
            &mut result
        },
        std::ptr::null_mut(),
    );
    if !result.query_records.is_null() {
        super::DnsFree(result.query_records, 1);
    }
    println!("StrictProbe_Status:\n{status}");
    ExitCode::SUCCESS
}

pub fn run(args: &[String]) -> ExitCode {
    super::print_runtime_marker();
    match args.first().map(String::as_str) {
        Some("ex-a") | Some("ex-w")
            if args.get(1).is_some_and(|s| {
                matches!(s.as_str(), "event" | "callback" | "namespace" | "provider")
                    || (args[0] == "ex-w" && s == "deadline")
            }) =>
        unsafe { ex(if args[0] == "ex-a" { "a" } else { "w" }, &args[1]) },
        Some("raw")
            if args
                .get(1)
                .is_some_and(|s| matches!(s.as_str(), "name" | "packet")) =>
        unsafe { raw(&args[1]) },
        Some("null-ex")
            if args
                .get(1)
                .is_some_and(|s| matches!(s.as_str(), "request" | "result")) =>
        unsafe { null_ex(&args[1]) },
        _ => {
            eprintln!("usage: --dns-strict ex-a|ex-w event|callback|namespace|provider; raw name|packet; null-ex request|result");
            ExitCode::FAILURE
        }
    }
}
