//! Native unsupported-entrypoint probes. Each command runs in its own process.
use super::{DnsQueryCancel, DnsQueryEx, DnsQueryRequest, DnsQueryResult};
use std::ffi::{c_void, CString};
use std::process::ExitCode;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use windows::core::{GUID, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Networking::WinSock::{
    freeaddrinfo, getaddrinfo, FreeAddrInfoEx, FreeAddrInfoExW, FreeAddrInfoW, GetAddrInfoExA,
    GetAddrInfoExCancel, GetAddrInfoExOverlappedResult, GetAddrInfoExW, GetAddrInfoW,
    WSAGetLastError, WSAStartup, ADDRINFOA, ADDRINFOEXA, ADDRINFOEXW, ADDRINFOW, TIMEVAL, WSADATA,
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

#[repr(C)]
struct ExReleaseObservation {
    done: HANDLE,
    callbacks: AtomicU32,
    status: AtomicI32,
}

#[repr(C)]
struct ExReleaseState {
    overlapped: OVERLAPPED,
    observation: *mut ExReleaseObservation,
    result: *mut ADDRINFOEXW,
    token: HANDLE,
}

// The callback owns the final caller storage release in this probe.  The
// Runtime must have completed every borrowed write before entering the
// callback and must not touch OVERLAPPED/result/name-handle after it returns.
unsafe extern "system" fn ex_completion_release(error: u32, _: u32, overlapped: *const OVERLAPPED) {
    let state_ptr = overlapped.cast_mut().cast::<ExReleaseState>();
    let state = &*state_ptr;
    let observation = &*state.observation;
    observation.status.store(error as i32, Ordering::Release);
    observation.callbacks.fetch_add(1, Ordering::AcqRel);
    if !state.result.is_null() {
        FreeAddrInfoExW(Some(state.result));
    }
    let _ = SetEvent(observation.done);
    drop(Box::from_raw(state_ptr));
}

unsafe fn ex_callback_release() -> ExitCode {
    let mut wsa = WSADATA::default();
    let startup = WSAStartup(0x202, &mut wsa);
    if startup != 0 {
        println!("StrictProbe_Startup:\n{startup}");
        return ExitCode::FAILURE;
    }
    let done = CreateEventW(None, true, false, None).expect("completion event");
    let observation = Box::new(ExReleaseObservation {
        done,
        callbacks: AtomicU32::new(0),
        status: AtomicI32::new(-1),
    });
    let observation_ptr = Box::into_raw(observation);
    let state = Box::new(ExReleaseState {
        overlapped: OVERLAPPED::default(),
        observation: observation_ptr,
        result: std::ptr::null_mut(),
        token: HANDLE::default(),
    });
    let state_ptr = Box::into_raw(state);
    let wide: Vec<u16> = "fixture.test".encode_utf16().chain(Some(0)).collect();
    let status = GetAddrInfoExW(
        PCWSTR(wide.as_ptr()),
        PCWSTR::null(),
        12,
        None,
        None,
        &mut (*state_ptr).result,
        None,
        Some(&(*state_ptr).overlapped as *const OVERLAPPED),
        Some(ex_completion_release),
        Some(&mut (*state_ptr).token as *mut HANDLE),
    );
    // The callback may already have freed state before submission returns.
    // Only the separately owned observation remains safe to inspect here;
    // pending OVERLAPPED state is covered by the non-releasing probe modes.
    println!("StrictProbe_Status:\n{status}");
    if status != 997 {
        // A conforming provider returns pending when it owns the callback.
        // Still avoid double-free if a provider completes inline before the
        // call returns and has already reclaimed the state.
        if (*observation_ptr).callbacks.load(Ordering::Acquire) == 0 {
            drop(Box::from_raw(state_ptr));
        }
        drop(Box::from_raw(observation_ptr));
        let _ = CloseHandle(done);
        let _ = windows::Win32::Networking::WinSock::WSACleanup();
        return ExitCode::FAILURE;
    }
    let wait = WaitForSingleObject(done, 10000);
    println!("StrictProbe_DrainEvent:\n{}", wait.0);
    if wait != WAIT_OBJECT_0 {
        // Keep observation and caller storage alive until a late callback, if
        // any.  This process exits with the allocations intentionally leaked.
        return ExitCode::FAILURE;
    }
    let callbacks = (*observation_ptr).callbacks.load(Ordering::Acquire);
    let callback_status = (*observation_ptr).status.load(Ordering::Acquire);
    println!(
        "StrictProbe_CallbackStatus:\n{callback_status}\nStrictProbe_FinalCallbacks:\n{callbacks}\nStrictProbe_Records:\n0"
    );
    let _ = CloseHandle(done);
    drop(Box::from_raw(observation_ptr));
    let _ = windows::Win32::Networking::WinSock::WSACleanup();
    ExitCode::SUCCESS
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
    let event_mode = mode == "event" || mode == "flags" || mode == "close-event";
    let callback_mode = mode == "callback" || mode == "cancel";
    let asynchronous = event_mode || callback_mode;
    if event_mode {
        state.overlapped.hEvent = done;
    }
    let name = CString::new("fixture.test").unwrap();
    let wide: Vec<u16> = "fixture.test".encode_utf16().chain(Some(0)).collect();
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
    let completion = if callback_mode {
        Some(ex_completion as unsafe extern "system" fn(u32, u32, *const OVERLAPPED))
    } else {
        None
    };
    let hints = if mode == "flags" {
        Some(ADDRINFOEXW {
            ai_flags: 0x400,
            ..ADDRINFOEXW::default()
        })
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
            hints.as_ref().map(|value| value as *const ADDRINFOEXW),
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
    let initial_overlapped = state.overlapped.Internal as u64;
    println!("StrictProbe_Status:\n{status}\nStrictProbe_LastError:\n{last_error}\nStrictProbe_InitialCallbacks:\n{initial_callbacks}\nStrictProbe_InitialEvent:\n{initial_event}\nStrictProbe_InitialToken:\n{initial_token}\nStrictProbe_InitialOverlappedStatus:\n{initial_overlapped}");
    let pending = status == 997;
    let mut completion_observed = !pending;
    let mut event_closed = false;
    if pending {
        let cancelled = if mode == "cancel" {
            GetAddrInfoExCancel(&state.token)
        } else {
            0
        };
        if mode == "cancel" {
            let wait = WaitForSingleObject(done, 10000);
            completion_observed = wait == WAIT_OBJECT_0;
            println!(
                "StrictProbe_Cancel:\n{cancelled}\nStrictProbe_DrainEvent:\n{}",
                wait.0
            );
        } else if mode == "close-event" {
            // The Runtime owns a duplicate of OVERLAPPED.hEvent.  Close the
            // caller's handle while the request is pending, then observe the
            // caller-owned OVERLAPPED publication and consume the result.
            let _ = CloseHandle(done);
            event_closed = true;
            let mut completed = false;
            for _ in 0..100 {
                if state.overlapped.Internal != 10036 {
                    completed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            completion_observed = completed;
            println!("StrictProbe_ClosedEventCompleted:\n{}", completed);
            if completion_observed {
                let first = GetAddrInfoExOverlappedResult(&state.overlapped);
                let second = GetAddrInfoExOverlappedResult(&state.overlapped);
                println!(
                    "StrictProbe_DrainStatus:\n{first}\nStrictProbe_DrainStatusSecond:\n{second}"
                );
            }
        } else if event_mode {
            let wait = WaitForSingleObject(done, 10000);
            completion_observed = wait == WAIT_OBJECT_0;
            println!("StrictProbe_DrainEvent:\n{}", wait.0);
            if completion_observed {
                let first = GetAddrInfoExOverlappedResult(&state.overlapped);
                let second = GetAddrInfoExOverlappedResult(&state.overlapped);
                println!(
                    "StrictProbe_DrainStatus:\n{first}\nStrictProbe_DrainStatusSecond:\n{second}"
                );
            }
        } else {
            let wait = WaitForSingleObject(done, 10000);
            completion_observed = wait == WAIT_OBJECT_0;
            println!("StrictProbe_DrainEvent:\n{}", wait.0);
        }
    } else {
        // Observe whether a supposedly synchronous rejection later produces a callback.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let final_callbacks = state.callbacks.load(Ordering::Acquire);
    let final_event = if event_closed {
        6
    } else {
        WaitForSingleObject(done, 0).0
    };
    let final_records = u32::from(!state.result_a.is_null() || !state.result_w.is_null());
    println!(
        "StrictProbe_CallbackStatus:\n{}\nStrictProbe_FinalCallbacks:\n{}\nStrictProbe_FinalEvent:\n{}\nStrictProbe_Records:\n{}",
        state.callback_status.load(Ordering::Acquire),
        final_callbacks,
        final_event,
        final_records
    );
    if pending && !completion_observed {
        // The caller-owned OVERLAPPED/result/token storage must remain alive
        // until completion.  Leak this probe state on timeout rather than
        // returning it to Rust while a provider may still call back into it.
        println!("StrictProbe_Lifetime:\nretained_after_timeout");
        let _ = Box::into_raw(state);
        return ExitCode::FAILURE;
    }
    if !state.result_a.is_null() {
        FreeAddrInfoEx(Some(state.result_a));
        state.result_a = std::ptr::null_mut();
    }
    if !state.result_w.is_null() {
        FreeAddrInfoExW(Some(state.result_w));
        state.result_w = std::ptr::null_mut();
    }
    if !event_closed {
        let _ = CloseHandle(done);
    }
    // Event mode has already polled the native terminal status.  Release all
    // caller-owned OVERLAPPED/result storage before the worker has necessarily
    // returned from its post-signal retirement path; the Runtime must perform
    // no further borrowed writes after publishing Internal.
    if pending && completion_observed && event_mode {
        drop(state);
        let _ = windows::Win32::Networking::WinSock::WSACleanup();
        return ExitCode::SUCCESS;
    }
    let _ = windows::Win32::Networking::WinSock::WSACleanup();
    ExitCode::SUCCESS
}

/// Synchronous GetAddrInfoEx A/W calls with an unsupported flag.  Profile
/// routing rejects the option explicitly; Host mode remains native.
unsafe fn ex_sync_flags(api: &str) -> ExitCode {
    let mut wsa = WSADATA::default();
    let startup = WSAStartup(0x202, &mut wsa);
    if startup != 0 {
        println!("StrictProbe_Startup:\n{startup}");
        return ExitCode::FAILURE;
    }
    let name = CString::new("fixture.test").unwrap();
    let wide: Vec<u16> = "fixture.test".encode_utf16().chain(Some(0)).collect();
    let mut result_a = std::ptr::null_mut();
    let mut result_w = std::ptr::null_mut();
    let status = if api == "a" {
        let hints = ADDRINFOEXA {
            ai_flags: 0x400,
            ..ADDRINFOEXA::default()
        };
        GetAddrInfoExA(
            PCSTR(name.as_ptr().cast()),
            PCSTR::null(),
            12,
            None,
            Some(&hints as *const ADDRINFOEXA),
            &mut result_a,
            None,
            None,
            None,
            None,
        )
    } else {
        let hints = ADDRINFOEXW {
            ai_flags: 0x400,
            ..ADDRINFOEXW::default()
        };
        GetAddrInfoExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            12,
            None,
            Some(&hints as *const ADDRINFOEXW),
            &mut result_w,
            None,
            None,
            None,
            None,
        )
    };
    if !result_a.is_null() {
        FreeAddrInfoEx(Some(result_a));
    }
    if !result_w.is_null() {
        FreeAddrInfoExW(Some(result_w));
    }
    println!(
        "StrictProbe_Status:\n{status}\nStrictProbe_InitialCallbacks:\n0\nStrictProbe_InitialEvent:\n258\nStrictProbe_InitialToken:\nfalse\nStrictProbe_FinalCallbacks:\n0\nStrictProbe_FinalEvent:\n258"
    );
    let _ = windows::Win32::Networking::WinSock::WSACleanup();
    ExitCode::SUCCESS
}

/// Synchronous getaddrinfo/getaddrinfoW calls with the same unsupported flag
/// mask.  This keeps the ordinary A/W hooks covered alongside GetAddrInfoEx.
unsafe fn basic_sync_flags(api: &str) -> ExitCode {
    let mut wsa = WSADATA::default();
    let startup = WSAStartup(0x202, &mut wsa);
    if startup != 0 {
        println!("StrictProbe_Startup:\n{startup}");
        return ExitCode::FAILURE;
    }
    let name = CString::new("fixture.test").unwrap();
    let wide: Vec<u16> = "fixture.test".encode_utf16().chain(Some(0)).collect();
    let mut result_a = std::ptr::null_mut();
    let mut result_w = std::ptr::null_mut();
    let status = if api == "a" {
        let hints = ADDRINFOA {
            ai_flags: 0x400,
            ..ADDRINFOA::default()
        };
        getaddrinfo(
            PCSTR(name.as_ptr().cast()),
            PCSTR::null(),
            Some(&hints as *const ADDRINFOA),
            &mut result_a,
        )
    } else {
        let hints = ADDRINFOW {
            ai_flags: 0x400,
            ..ADDRINFOW::default()
        };
        GetAddrInfoW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            Some(&hints as *const ADDRINFOW),
            &mut result_w,
        )
    };
    if !result_a.is_null() {
        freeaddrinfo(Some(result_a));
    }
    if !result_w.is_null() {
        FreeAddrInfoW(Some(result_w));
    }
    println!(
        "StrictProbe_Status:\n{status}\nStrictProbe_InitialCallbacks:\n0\nStrictProbe_InitialEvent:\n258\nStrictProbe_InitialToken:\nfalse\nStrictProbe_FinalCallbacks:\n0\nStrictProbe_FinalEvent:\n258"
    );
    let _ = windows::Win32::Networking::WinSock::WSACleanup();
    ExitCode::SUCCESS
}

/// Run event-mode requests without calling GetAddrInfoExOverlappedResult.
/// The Runtime must retire each completed event operation after signaling it;
/// otherwise the bounded pending map would reject the 65th request.
unsafe fn ex_repeat() -> ExitCode {
    let mut wsa = WSADATA::default();
    let startup = WSAStartup(0x202, &mut wsa);
    if startup != 0 {
        println!("StrictProbe_Startup:\n{startup}");
        return ExitCode::FAILURE;
    }
    let wide: Vec<u16> = "fixture.test".encode_utf16().chain(Some(0)).collect();
    let mut completed = 0_u32;
    for _ in 0..80 {
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
        state.overlapped.hEvent = done;
        let status = GetAddrInfoExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            12,
            None,
            None,
            &mut state.result_w,
            None,
            Some(&state.overlapped as *const OVERLAPPED),
            None,
            Some(&mut state.token as *mut HANDLE),
        );
        let wait = WaitForSingleObject(done, 10000);
        if status != 997 || wait != WAIT_OBJECT_0 {
            println!("StrictProbe_RepeatStatus:\n{status}");
            if status == 997 && wait != WAIT_OBJECT_0 {
                // Keep caller-owned storage alive if the worker has not
                // completed yet; the process can then terminate safely
                // without returning a borrowed OVERLAPPED to Rust.
                let _ = Box::into_raw(state);
                return ExitCode::FAILURE;
            }
            let _ = CloseHandle(done);
            return ExitCode::FAILURE;
        }
        if !state.result_w.is_null() {
            FreeAddrInfoExW(Some(state.result_w));
        }
        let _ = CloseHandle(done);
        completed += 1;
    }
    println!("StrictProbe_RepeatCompleted:\n{completed}");
    let _ = windows::Win32::Networking::WinSock::WSACleanup();
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
    // Keep every caller-owned buffer in a heap allocation.  A native Raw
    // provider may retain these buffers until its callback; this lets the
    // timeout path leak a coherent request instead of returning dangling
    // stack/Vec storage to an in-flight operation.
    let mut name = Box::new(
        "strict.fixture.test"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>(),
    );
    let mut packet = Box::new(vec![
        0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 6, b's', b't', b'r', b'i', b'c', b't', 7, b'f',
        b'i', b'x', b't', b'u', b'r', b'e', 4, b't', b'e', b's', b't', 0, 0, 1, 0, 1,
    ]);
    let mut token = Box::new(DnsQueryCancel { reserved: [0; 32] });
    let mut request = Box::new(RawRequest {
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
    });
    let status = query(request.as_mut(), token.as_mut());
    let initial_callbacks = state.callbacks.load(Ordering::Acquire);
    let initial_event = WaitForSingleObject(done, 0).0;
    let token_changed = token.reserved.iter().any(|byte| *byte != 0);
    println!("StrictProbe_Status:\n{status}\nStrictProbe_InitialCallbacks:\n{initial_callbacks}\nStrictProbe_InitialEvent:\n{initial_event}\nStrictProbe_InitialToken:\n{token_changed}");
    let pending = status == 9506;
    if pending {
        let cancelled = cancel(token.as_mut());
        let wait = WaitForSingleObject(done, 10000).0;
        println!("StrictProbe_Cancel:\n{cancelled}\nStrictProbe_DrainEvent:\n{wait}");
        if wait != WAIT_OBJECT_0.0 {
            println!("StrictProbe_Lifetime:\nretained_after_timeout");
            let _ = Box::into_raw(state);
            let _ = Box::into_raw(name);
            let _ = Box::into_raw(packet);
            let _ = Box::into_raw(token);
            let _ = Box::into_raw(request);
            return ExitCode::FAILURE;
        }
    } else {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    println!(
        "StrictProbe_FinalCallbacks:\n{}\nStrictProbe_FinalEvent:\n{}",
        state.callbacks.load(Ordering::Acquire),
        WaitForSingleObject(done, 0).0
    );
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
        Some("ex-w") if args.get(1).is_some_and(|s| s == "repeat") => unsafe { ex_repeat() },
        Some("ex-w") if args.get(1).is_some_and(|s| s == "callback-free") => unsafe {
            ex_callback_release()
        },
        Some("ex-a") | Some("ex-w") if args.get(1).is_some_and(|s| s == "sync-flags") => unsafe {
            ex_sync_flags(if args[0] == "ex-a" { "a" } else { "w" })
        },
        Some("gai-a") | Some("gai-w") if args.get(1).is_some_and(|s| s == "sync-flags") => unsafe {
            basic_sync_flags(if args[0] == "gai-a" { "a" } else { "w" })
        },
        Some("ex-a") | Some("ex-w")
            if args.get(1).is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "event"
                        | "close-event"
                        | "callback"
                        | "callback-free"
                        | "cancel"
                        | "flags"
                        | "sync-flags"
                        | "namespace"
                        | "provider"
                ) || (args[0] == "ex-w" && s == "deadline")
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
            eprintln!("usage: --dns-strict ex-a|ex-w event|close-event|callback|callback-free|cancel|flags|sync-flags|repeat|namespace|provider; gai-a|gai-w sync-flags; raw name|packet; null-ex request|result");
            ExitCode::FAILURE
        }
    }
}
