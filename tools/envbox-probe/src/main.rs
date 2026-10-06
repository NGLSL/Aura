//! envbox-probe: Host / Profile environment snapshot for EnvBox acceptance.

use envbox_probe::collect_host_snapshot;
use std::ffi::c_void;
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;

mod as_user;
mod dns_rr;
mod dns_strict;
mod with_token;

#[repr(C)]
struct DnsQueryRequest {
    version: u32,
    query_name: *const u16,
    query_type: u16,
    query_options: u64,
    dns_server_list: *mut c_void,
    interface_index: u32,
    completion: Option<unsafe extern "system" fn(*mut c_void, *mut DnsQueryResult)>,
    query_context: *mut c_void,
}

#[repr(C)]
struct DnsQueryResult {
    version: u32,
    query_status: i32,
    query_options: u64,
    query_records: *mut c_void,
    reserved: *mut c_void,
}

// The public DNS_RECORD header uses pointer-sized pNext/pName fields.  Keep
// this repr(C) view shared by the async A-record consumer so x86 reads the
// same wire record at wType/data offsets 8/24 that x64 reads at 16/32.
#[repr(C)]
struct DnsRecordHeader {
    next: *mut DnsRecordHeader,
    name: *mut c_void,
    kind: u16,
    length: u16,
    flags: u32,
    ttl: u32,
    reserved: u32,
    data: [u8; 0],
}

#[derive(Clone, Copy)]
#[repr(C, align(8))]
struct DnsQueryCancel {
    reserved: [u8; 32],
}

#[link(name = "dnsapi")]
extern "system" {
    fn DnsQueryEx(
        request: *const DnsQueryRequest,
        result: *mut DnsQueryResult,
        cancel: *mut DnsQueryCancel,
    ) -> i32;
    fn DnsCancelQuery(cancel: *const DnsQueryCancel) -> i32;
    fn DnsFree(ptr: *mut c_void, free_type: u32);
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(
        attributes: *mut c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn SetEvent(event: *mut c_void) -> i32;
    fn WaitForSingleObject(event: *mut c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

struct AsyncDnsState {
    event: *mut c_void,
    reentry_event: *mut c_void,
    query_name: Vec<u16>,
    cancel: DnsQueryCancel,
    result: DnsQueryResult,
    query_status: AtomicI32,
    callback_cancel_status: AtomicI32,
    callback_count: AtomicU32,
    addresses: Mutex<Vec<String>>,
    reenter: bool,
    inspect_pending: bool,
    reentry_result: DnsQueryResult,
    reentry_started: AtomicU32,
    reentry_return_status: AtomicI32,
    reentry_initial_query_status: AtomicI32,
    reentry_query_status: AtomicI32,
    reentry_callback_count: AtomicU32,
    stale_cancel_status: AtomicI32,
    reentry_addresses: Mutex<Vec<String>>,
}

// The state is allocated by the Probe and remains alive until the completion
// event has fired. The completion callback runs in another thread context.
unsafe impl Send for AsyncDnsState {}
unsafe impl Sync for AsyncDnsState {}

unsafe fn consume_dns_result(results: *mut DnsQueryResult, addresses: &Mutex<Vec<String>>) -> i32 {
    if results.is_null() {
        return -1;
    }
    let status = (*results).query_status;
    if !(*results).query_records.is_null() {
        let mut ips = Vec::new();
        let mut record = (*results).query_records.cast::<DnsRecordHeader>();
        while !record.is_null() {
            let record_ref = &*record;
            if record_ref.kind == 1 {
                let data = record_ref.data.as_ptr();
                let bytes = [*data, *data.add(1), *data.add(2), *data.add(3)];
                ips.push(format!(
                    "{}.{}.{}.{}",
                    bytes[0], bytes[1], bytes[2], bytes[3]
                ));
            }
            record = record_ref.next;
        }
        if let Ok(mut stored) = addresses.lock() {
            *stored = ips;
        }
        let records = (*results).query_records;
        DnsFree(records, 1);
        (*results).query_records = std::ptr::null_mut();
    }
    status
}

unsafe fn start_reentry(state: *mut AsyncDnsState) {
    const DNS_TYPE_A: u16 = 1;
    const DNS_QUERY_REQUEST_VERSION1: u32 = 1;
    const DNS_QUERY_RESULTS_VERSION1: u32 = 1;
    const DNS_REQUEST_PENDING: i32 = 9506;

    let request = DnsQueryRequest {
        version: DNS_QUERY_REQUEST_VERSION1,
        query_name: (*state).query_name.as_ptr(),
        query_type: DNS_TYPE_A,
        query_options: 0,
        dns_server_list: std::ptr::null_mut(),
        interface_index: 0,
        completion: Some(dns_query_ex_reentry_completion),
        query_context: state as *mut c_void,
    };
    (*state).reentry_result = DnsQueryResult {
        version: DNS_QUERY_RESULTS_VERSION1,
        query_status: 0,
        query_options: 0,
        query_records: std::ptr::null_mut(),
        reserved: std::ptr::null_mut(),
    };
    let return_status = DnsQueryEx(
        &request,
        std::ptr::addr_of_mut!((*state).reentry_result),
        std::ptr::addr_of_mut!((*state).cancel),
    );
    // Observe the field after the FFI call. A fast completion may already
    // have replaced DNS_REQUEST_PENDING with its final status; do not infer
    // this value from the function return code.
    let initial_status = if (*state).inspect_pending {
        AtomicI32::from_ptr(std::ptr::addr_of_mut!((*state).reentry_result.query_status))
            .load(Ordering::Acquire)
    } else {
        -1
    };
    (*state)
        .reentry_return_status
        .store(return_status, Ordering::Release);
    (*state)
        .reentry_initial_query_status
        .store(initial_status, Ordering::Release);

    if return_status != DNS_REQUEST_PENDING
        && (*state).reentry_callback_count.load(Ordering::Acquire) == 0
    {
        let status = consume_dns_result(
            std::ptr::addr_of_mut!((*state).reentry_result),
            &(*state).reentry_addresses,
        );
        (*state)
            .reentry_query_status
            .store(status, Ordering::Release);
        if !(*state).reentry_event.is_null() {
            SetEvent((*state).reentry_event);
        }
    }
}

unsafe extern "system" fn dns_query_ex_completion(
    context: *mut c_void,
    results: *mut DnsQueryResult,
) {
    if context.is_null() {
        return;
    }
    let state = context as *mut AsyncDnsState;
    let status = consume_dns_result(results, &(*state).addresses);
    (*state).query_status.store(status, Ordering::Release);
    if (*state).inspect_pending {
        // Completion leaves the old opaque token as a local tombstone.  A
        // cancel from inside this callback must therefore stop at the Runtime
        // with ERROR_INVALID_PARAMETER (87), never reach the Windows provider.
        // Host control calls omit this extra cancellation so their native
        // callback contract is not changed by the Probe.
        let callback_cancel = DnsCancelQuery(&(*state).cancel);
        (*state)
            .callback_cancel_status
            .store(callback_cancel, Ordering::Release);
    }
    (*state).callback_count.fetch_add(1, Ordering::AcqRel);
    if (*state).inspect_pending
        && (*state).reenter
        && (*state)
            .reentry_started
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        // Preserve the completed generation before re-entry overwrites the
        // shared cancel storage.  The stale copy must remain locally rejected
        // and must not cancel the newly published generation.
        let stale_cancel = (*state).cancel;
        start_reentry(state);
        let stale_status = DnsCancelQuery(&stale_cancel);
        (*state)
            .stale_cancel_status
            .store(stale_status, Ordering::Release);
    }
    SetEvent((*state).event);
}

unsafe extern "system" fn dns_query_ex_reentry_completion(
    context: *mut c_void,
    results: *mut DnsQueryResult,
) {
    if context.is_null() {
        return;
    }
    let state = context as *mut AsyncDnsState;
    let status = consume_dns_result(results, &(*state).reentry_addresses);
    (*state)
        .reentry_query_status
        .store(status, Ordering::Release);
    (*state)
        .reentry_callback_count
        .fetch_add(1, Ordering::AcqRel);
    if !(*state).reentry_event.is_null() {
        SetEvent((*state).reentry_event);
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--identity-json") {
        let mut values: std::collections::BTreeMap<String, String> =
            envbox_probe::identity::collect()
                .fields
                .into_iter()
                .map(|field| (field.name, field.value))
                .collect();
        values.insert("RuntimeLoaded".into(), runtime_loaded().to_string());
        println!(
            "{}",
            serde_json::to_string(&values).expect("identity strings serialize")
        );
        if let Some(index) = args.iter().position(|arg| arg == "--identity-child") {
            let Some(path) = args.get(index + 1) else {
                return ExitCode::FAILURE;
            };
            // Child propagation must restore immutable Profile identity rather than
            // trust a caller's rewritten inherited environment.
            let status = Command::new(path)
                .arg("--identity-json")
                .env("ENVBOX_IDENTITY_COMPUTER_NAME", "POISON")
                .env("ENVBOX_IDENTITY_USER_NAME", "POISON")
                .env("COMPUTERNAME", "POISON")
                .env("USERNAME", "POISON")
                .status();
            return match status {
                Ok(status) if status.success() => ExitCode::SUCCESS,
                _ => ExitCode::FAILURE,
            };
        }
        return ExitCode::SUCCESS;
    }
    if let Some(index) = args.iter().position(|arg| arg == "--udp-send-to") {
        let Some(target) = args
            .get(index + 1)
            .and_then(|arg| arg.parse::<std::net::SocketAddr>().ok())
        else {
            eprintln!("usage: --udp-send-to literalSocketAddr");
            return ExitCode::FAILURE;
        };
        let bind = if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        match std::net::UdpSocket::bind(bind)
            .and_then(|socket| socket.send_to(b"aura-endpoint-fixture", target))
        {
            Ok(bytes) => println!("UdpSend_Bytes:\n{bytes}\nUdpSend_Error:\n0"),
            Err(error) => println!(
                "UdpSend_Bytes:\n0\nUdpSend_Error:\n{}",
                error.raw_os_error().unwrap_or(-1)
            ),
        }
        return ExitCode::SUCCESS;
    }
    if let Some(index) = args.iter().position(|arg| arg == "--dns-strict") {
        return dns_strict::run(&args[index + 1..]);
    }
    if let Some(index) = args.iter().position(|arg| arg == "--dns-rr") {
        return dns_rr::run(&args[index + 1..]);
    }
    let spawn_child = args.iter().any(|a| a == "--spawn-child");
    if args.iter().any(|a| a == "--with-token-boundary") {
        return with_token::run();
    }
    let spawn_as_user_child = args.iter().any(|a| a == "--spawn-as-user-child");
    let is_child = args.iter().any(|a| a == "--child");

    if let Some(path) = args.iter().position(|a| a == "--as-user-child-output") {
        let Some(path) = args.get(path + 1) else {
            eprintln!("--as-user-child-output requires a path");
            return ExitCode::FAILURE;
        };
        return match std::fs::write(path, child_output()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("as-user child output failed: {error}");
                ExitCode::FAILURE
            }
        };
    }

    // Optional DNS routing probe: `--resolve <name>` prints getaddrinfo results.
    // Optional DnsQuery_A smoke: `--resolve-dnsquery <name>`.
    // Optional synchronous DnsQueryEx smoke: `--resolve-dnsquery-ex <name>`.
    // Optional asynchronous DnsQueryEx smoke: `--resolve-dnsquery-ex-async <name>`.
    // Add `--cancel` to exercise DnsCancelQuery and callback lifetime. The
    // async probe also accepts `--copy-cancel` and `--reenter` to exercise the
    // opaque cancel token and callback re-entry contracts. The acceptance
    // fixture adds `--inspect-pending-status` while it holds the DNS response;
    // generic/native async probes do not read a result being written by a
    // provider whose synchronization is outside Probe's control.
    let mut resolve_name: Option<String> = None;
    let mut resolve_dnsquery: Option<String> = None;
    let mut resolve_dnsquery_ex: Option<String> = None;
    let mut resolve_dnsquery_ex_async: Option<String> = None;
    let mut resolve_dnsquery_ex_async_cancel = false;
    let mut resolve_dnsquery_ex_async_copy_cancel = false;
    let mut resolve_dnsquery_ex_async_reenter = false;
    let mut inspect_pending_status = false;
    let dns_system_settings = args.iter().any(|a| a == "--dns-system-settings");
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--resolve" && i + 1 < args.len() {
            resolve_name = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--resolve-dnsquery" && i + 1 < args.len() {
            resolve_dnsquery = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--resolve-dnsquery-ex" && i + 1 < args.len() {
            resolve_dnsquery_ex = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--resolve-dnsquery-ex-async" && i + 1 < args.len() {
            resolve_dnsquery_ex_async = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--cancel" {
            resolve_dnsquery_ex_async_cancel = true;
            i += 1;
        } else if args[i] == "--copy-cancel" {
            resolve_dnsquery_ex_async_copy_cancel = true;
            i += 1;
        } else if args[i] == "--reenter" {
            resolve_dnsquery_ex_async_reenter = true;
            i += 1;
        } else if args[i] == "--inspect-pending-status" {
            inspect_pending_status = true;
            i += 1;
        } else {
            i += 1;
        }
    }

    if is_child {
        print_child();
        return ExitCode::SUCCESS;
    }

    if let Some(name) = resolve_name {
        print_runtime_marker();
        print_resolve(&name);
        return ExitCode::SUCCESS;
    }
    if let Some(name) = resolve_dnsquery {
        print_runtime_marker();
        print_resolve_dnsquery(&name);
        return ExitCode::SUCCESS;
    }
    if let Some(name) = resolve_dnsquery_ex {
        print_runtime_marker();
        print_resolve_dnsquery_ex(&name);
        return ExitCode::SUCCESS;
    }
    if let Some(name) = resolve_dnsquery_ex_async {
        print_runtime_marker();
        print_resolve_dnsquery_ex_async(
            &name,
            resolve_dnsquery_ex_async_cancel,
            resolve_dnsquery_ex_async_copy_cancel,
            resolve_dnsquery_ex_async_reenter,
            inspect_pending_status,
        );
        return ExitCode::SUCCESS;
    }
    if dns_system_settings {
        print_runtime_marker();
        print_dns_system_settings();
        return ExitCode::SUCCESS;
    }

    print_parent();
    if spawn_child {
        spawn_child_probe();
    }
    if spawn_as_user_child {
        println!("=== CREATEPROCESSASUSERW ===");
        match as_user::spawn_child() {
            Ok(as_user::Outcome::Succeeded(output)) => {
                println!("Status: succeeded");
                print!("{output}");
            }
            Ok(as_user::Outcome::SkippedPrivilege(stage, error)) => {
                println!("Status: skipped ({stage}: Windows privilege error {error})");
            }
            Err(error) => {
                eprintln!("CreateProcessAsUserW probe failed: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// Ticket 26: stable RESOLVE section for DNS routing acceptance.
fn print_resolve(name: &str) {
    use std::net::ToSocketAddrs;
    println!("=== RESOLVE ===");
    println!("Name:");
    println!("{name}");
    println!();
    println!("getaddrinfo:");
    match (name, 0u16).to_socket_addrs() {
        Ok(addrs) => {
            let mut ips: Vec<String> = Vec::new();
            for a in addrs {
                let ip = a.ip().to_string();
                if !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
            if ips.is_empty() {
                println!("<empty>");
            } else {
                println!("{}", ips.join(", "));
            }
        }
        Err(err) => {
            let code = err
                .raw_os_error()
                .filter(|code| *code != 0)
                .unwrap_or_else(|| raw_getaddrinfo_status(name));
            if code == 0 {
                println!("<error>");
            } else {
                println!("<error {code}>");
            }
        }
    }
    println!();
}

/// Rust's Windows ToSocketAddrs conversion can erase the EAI status and leave
/// raw_os_error() as zero. Query the native entry point for the exact code so
/// DNS routing acceptance can distinguish EAI_NONAME from a generic failure.
fn raw_getaddrinfo_status(name: &str) -> i32 {
    use std::ffi::c_void;

    #[link(name = "ws2_32")]
    extern "system" {
        fn getaddrinfo(
            node: *const u8,
            service: *const u8,
            hints: *const c_void,
            result: *mut *mut c_void,
        ) -> i32;
        fn freeaddrinfo(result: *mut c_void);
    }

    let mut node = name.as_bytes().to_vec();
    node.push(0);
    let service = b"0\0";
    unsafe {
        let mut result = std::ptr::null_mut();
        let status = getaddrinfo(
            node.as_ptr(),
            service.as_ptr(),
            std::ptr::null(),
            &mut result,
        );
        if !result.is_null() {
            freeaddrinfo(result);
        }
        status
    }
}

/// Optional DnsQuery_A smoke (DNS routing review).
fn print_resolve_dnsquery(name: &str) {
    println!("=== RESOLVE DNSQUERY ===");
    println!("Name:");
    println!("{name}");
    println!();
    println!("DnsQuery_A:");
    match dns_query_a(name) {
        Ok(ips) if ips.is_empty() => println!("<empty>"),
        Ok(ips) => println!("{}", ips.join(", ")),
        Err(status) => println!("<error {status}>"),
    }
    println!();
}

/// Optional synchronous DnsQueryEx smoke. A null completion callback makes
/// DnsQueryEx wait for the result and return the DNS_RECORD list directly.
fn print_resolve_dnsquery_ex(name: &str) {
    println!("=== RESOLVE DNSQUERY EX ===");
    println!("Name:");
    println!("{name}");
    println!();
    println!("DnsQueryEx_A:");
    match dns_query_ex_a(name) {
        Ok(ips) if ips.is_empty() => println!("<empty>"),
        Ok(ips) => println!("{}", ips.join(", ")),
        Err(status) => println!("<error {status}>"),
    }
    println!();
}

struct AsyncDnsOutcome {
    return_status: i32,
    initial_query_status: i32,
    callback_status: i32,
    callback_cancel_status: i32,
    callback_count: u32,
    cancel_status: Option<i32>,
    cancel_copied: bool,
    wait_status: u32,
    addresses: Vec<String>,
    reentry_return_status: Option<i32>,
    reentry_initial_query_status: Option<i32>,
    reentry_query_status: Option<i32>,
    reentry_callback_count: Option<u32>,
    stale_cancel_status: Option<i32>,
    reentry_wait_status: Option<u32>,
    reentry_addresses: Option<Vec<String>>,
}

/// Exercise the asynchronous DnsQueryEx contract. The Runtime hook owns a
/// bounded worker for Profile-routed VirtualView requests; the Probe keeps the
/// result, cancel storage, and user context alive until the callback fires.
fn print_resolve_dnsquery_ex_async(
    name: &str,
    cancel: bool,
    copy_cancel: bool,
    reenter: bool,
    inspect_pending: bool,
) {
    println!("=== RESOLVE DNSQUERY EX ASYNC ===");
    println!("Name:");
    println!("{name}");
    println!();
    let outcome = dns_query_ex_a_async(name, cancel, copy_cancel, reenter, inspect_pending);
    println!("DnsQueryEx_A_Async:");
    if outcome.callback_count > 0 && outcome.callback_status == 0 {
        if outcome.addresses.is_empty() {
            println!("<empty>");
        } else {
            println!("{}", outcome.addresses.join(", "));
        }
    } else if outcome.callback_count > 0 {
        println!("<error {}>", outcome.callback_status);
    } else {
        println!("<error {}>", outcome.return_status);
    }
    println!();
    println!("DnsQueryEx_A_Async_ReturnStatus:");
    println!("{}", outcome.return_status);
    println!();
    println!("DnsQueryEx_A_Async_InitialQueryStatus:");
    println!("{}", outcome.initial_query_status);
    println!();
    println!("DnsQueryEx_A_Async_CallbackStatus:");
    println!("{}", outcome.callback_status);
    println!();
    if outcome.callback_cancel_status >= 0 {
        println!("DnsQueryEx_A_Async_CallbackCancelStatus:");
        println!("{}", outcome.callback_cancel_status);
        println!();
    }
    println!("DnsQueryEx_A_Async_Callbacks:");
    println!("{}", outcome.callback_count);
    println!();
    println!("DnsQueryEx_A_Async_WaitStatus:");
    println!("{}", outcome.wait_status);
    println!();
    if let Some(status) = outcome.cancel_status {
        println!("DnsQueryEx_A_Async_CancelStatus:");
        println!("{status}");
        println!();
    }
    if outcome.cancel_copied {
        println!("DnsQueryEx_A_Async_CancelCopied:");
        println!("1");
        println!();
    }
    if let Some(status) = outcome.reentry_return_status {
        println!("DnsQueryEx_A_Async_ReentryReturnStatus:");
        println!("{status}");
        println!();
    }
    if let Some(status) = outcome.reentry_initial_query_status {
        println!("DnsQueryEx_A_Async_ReentryInitialQueryStatus:");
        println!("{status}");
        println!();
    }
    if let Some(status) = outcome.reentry_query_status {
        println!("DnsQueryEx_A_Async_ReentryCallbackStatus:");
        println!("{status}");
        println!();
    }
    if let Some(count) = outcome.reentry_callback_count {
        println!("DnsQueryEx_A_Async_ReentryCallbacks:");
        println!("{count}");
        println!();
    }
    if let Some(status) = outcome.stale_cancel_status {
        println!("DnsQueryEx_A_Async_StaleCancelStatus:");
        println!("{status}");
        println!();
    }
    if let Some(status) = outcome.reentry_wait_status {
        println!("DnsQueryEx_A_Async_ReentryWaitStatus:");
        println!("{status}");
        println!();
    }
    if let Some(addresses) = outcome.reentry_addresses {
        println!("DnsQueryEx_A_Async_Reentry:");
        if addresses.is_empty() {
            println!("<empty>");
        } else {
            println!("{}", addresses.join(", "));
        }
        println!();
    }
}

/// Probe the registry values Chromium reads while building its Windows DNS
/// system-settings snapshot.  This is intentionally a read-only probe: it
/// makes the Profile-vs-Host behavior visible without changing host policy.
fn print_dns_system_settings() {
    println!("=== DNS SYSTEM SETTINGS ===");
    let entries = [
        (
            r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters",
            &[
                "SearchList",
                "Domain",
                "UseDomainNameDevolution",
                "DomainNameDevolutionLevel",
            ][..],
        ),
        (
            r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters",
            &[
                "SearchList",
                "Domain",
                "UseDomainNameDevolution",
                "DomainNameDevolutionLevel",
            ][..],
        ),
        (
            r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters",
            &["UseDomainNameDevolution", "DomainNameDevolutionLevel"][..],
        ),
        (
            r"SOFTWARE\Policies\Microsoft\Windows NT\DNSClient",
            &[
                "SearchList",
                "UseDomainNameDevolution",
                "DomainNameDevolutionLevel",
                "AppendToMultiLabelName",
            ][..],
        ),
        (
            r"SOFTWARE\Policies\Microsoft\System\DNSClient",
            &["PrimaryDnsSuffix"][..],
        ),
        (
            r"SOFTWARE\Policies\Microsoft\Windows NT\DNSClient\DnsPolicyConfig",
            &[][..],
        ),
        (
            r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig",
            &[][..],
        ),
        (
            r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsConnections",
            &[][..],
        ),
        (
            r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsConnectionsProxies",
            &[][..],
        ),
    ];
    for (key, values) in entries {
        println!("Key:");
        println!("HKLM\\{key}");
        if values.is_empty() {
            println!("Open:");
            println!("{}", dns_registry_key_state(key));
            println!();
            continue;
        }
        for value in values {
            println!("{value}:");
            println!("{}", dns_registry_value(key, value));
        }
        println!();
    }
}

#[cfg(windows)]
fn dns_registry_key_state(path: &str) -> String {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    type Hkey = *mut c_void;
    const HKEY_LOCAL_MACHINE: Hkey = -2147483646isize as Hkey;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hkey: Hkey,
            subkey: *const u16,
            options: u32,
            sam_desired: u32,
            result: *mut Hkey,
        ) -> i32;
        fn RegCloseKey(hkey: Hkey) -> i32;
    }

    let wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut key: Hkey = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            wide.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        );
        if status == ERROR_SUCCESS {
            let _ = RegCloseKey(key);
            "present".to_string()
        } else if status == ERROR_FILE_NOT_FOUND {
            "<missing>".to_string()
        } else {
            format!("<error {status}>")
        }
    }
}

#[cfg(not(windows))]
fn dns_registry_key_state(_path: &str) -> String {
    "<unsupported>".to_string()
}

#[cfg(windows)]
fn dns_registry_value(path: &str, name: &str) -> String {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    type Hkey = *mut c_void;
    const HKEY_LOCAL_MACHINE: Hkey = -2147483646isize as Hkey;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const REG_DWORD: u32 = 4;
    const REG_MULTI_SZ: u32 = 7;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hkey: Hkey,
            subkey: *const u16,
            options: u32,
            sam_desired: u32,
            result: *mut Hkey,
        ) -> i32;
        fn RegQueryValueExW(
            hkey: Hkey,
            value_name: *const u16,
            reserved: *mut u32,
            value_type: *mut u32,
            data: *mut u8,
            data_size: *mut u32,
        ) -> i32;
        fn RegCloseKey(hkey: Hkey) -> i32;
    }

    let key_wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let name_wide: Vec<u16> = OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut key: Hkey = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            key_wide.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        );
        if status != ERROR_SUCCESS {
            return if status == ERROR_FILE_NOT_FOUND {
                "<missing>".to_string()
            } else {
                format!("<error {status}>")
            };
        }
        let mut value_type = 0;
        let mut size = 0;
        let status = RegQueryValueExW(
            key,
            name_wide.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            std::ptr::null_mut(),
            &mut size,
        );
        if status != ERROR_SUCCESS {
            let _ = RegCloseKey(key);
            return if status == ERROR_FILE_NOT_FOUND {
                "<missing>".to_string()
            } else {
                format!("<error {status}>")
            };
        }
        let mut data = vec![0u8; size as usize];
        let status = RegQueryValueExW(
            key,
            name_wide.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            data.as_mut_ptr(),
            &mut size,
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            return format!("<error {status}>");
        }
        if matches!(value_type, REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ) {
            let words = data[..size as usize]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|word| *word != 0)
                .collect::<Vec<_>>();
            let value = String::from_utf16_lossy(&words);
            return if value.is_empty() {
                "<empty>".to_string()
            } else {
                value
            };
        }
        if value_type == REG_DWORD && data.len() >= 4 {
            return format!(
                "{}",
                u32::from_le_bytes([data[0], data[1], data[2], data[3]])
            );
        }
        format!("<type {value_type}, {size} bytes>")
    }
}

#[cfg(not(windows))]
fn dns_registry_value(_path: &str, _name: &str) -> String {
    "<unsupported>".to_string()
}

/// Minimal DnsQuery_A via dnsapi (A records only). status 0 on success.
fn dns_query_a(name: &str) -> Result<Vec<String>, i32> {
    use std::ffi::c_void;

    #[link(name = "dnsapi")]
    extern "system" {
        fn DnsQuery_A(
            name: *const u8,
            wtype: u16,
            options: u32,
            extra: *mut c_void,
            out: *mut *mut c_void,
            reserved: *mut *mut c_void,
        ) -> i32;
        fn DnsFree(ptr: *mut c_void, free_type: u32);
    }

    const DNS_TYPE_A: u16 = 1;
    const DNS_FREE_RECORD_LIST: u32 = 1;

    let mut cname: Vec<u8> = name.bytes().collect();
    cname.push(0);
    unsafe {
        let mut out: *mut c_void = std::ptr::null_mut();
        let st = DnsQuery_A(
            cname.as_ptr(),
            DNS_TYPE_A,
            0,
            std::ptr::null_mut(),
            &mut out,
            std::ptr::null_mut(),
        );
        if st != 0 {
            return Err(st);
        }
        let mut ips: Vec<String> = Vec::new();
        let mut rec = out;
        // DNS_RECORD x64: pNext@0 pName@8 wType@16 ... Data.A@32
        while !rec.is_null() {
            let base = rec as *const u8;
            let wtype = u16::from_le_bytes([*base.add(16), *base.add(17)]);
            if wtype == DNS_TYPE_A {
                let b = [*base.add(32), *base.add(33), *base.add(34), *base.add(35)];
                ips.push(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]));
            }
            rec = *(rec as *const *mut c_void);
        }
        if !out.is_null() {
            DnsFree(out, DNS_FREE_RECORD_LIST);
        }
        Ok(ips)
    }
}

/// Minimal synchronous DnsQueryEx via dnsapi (A records only). status 0 on
/// success. This is intentionally kept as an FFI probe so the acceptance test
/// exercises the same API that Chromium and other modern Windows clients use.
fn dns_query_ex_a(name: &str) -> Result<Vec<String>, i32> {
    const DNS_TYPE_A: u16 = 1;
    const DNS_QUERY_REQUEST_VERSION1: u32 = 1;
    const DNS_QUERY_RESULTS_VERSION1: u32 = 1;
    const DNS_FREE_RECORD_LIST: u32 = 1;

    let query_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let request = DnsQueryRequest {
        version: DNS_QUERY_REQUEST_VERSION1,
        query_name: query_name.as_ptr(),
        query_type: DNS_TYPE_A,
        query_options: 0,
        dns_server_list: std::ptr::null_mut(),
        interface_index: 0,
        completion: None,
        query_context: std::ptr::null_mut(),
    };
    let mut result = DnsQueryResult {
        version: DNS_QUERY_RESULTS_VERSION1,
        query_status: 0,
        query_options: 0,
        query_records: std::ptr::null_mut(),
        reserved: std::ptr::null_mut(),
    };

    unsafe {
        let st = DnsQueryEx(&request, &mut result, std::ptr::null_mut());
        if st != 0 {
            return Err(st);
        }
        if result.query_status != 0 {
            if !result.query_records.is_null() {
                DnsFree(result.query_records, DNS_FREE_RECORD_LIST);
            }
            return Err(result.query_status);
        }
        let mut ips: Vec<String> = Vec::new();
        let mut rec = result.query_records;
        // DNS_RECORD x64: pNext@0 pName@8 wType@16 ... Data.A@32
        while !rec.is_null() {
            let base = rec as *const u8;
            let wtype = u16::from_le_bytes([*base.add(16), *base.add(17)]);
            if wtype == DNS_TYPE_A {
                let b = [*base.add(32), *base.add(33), *base.add(34), *base.add(35)];
                ips.push(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]));
            }
            rec = *(rec as *const *mut c_void);
        }
        if !result.query_records.is_null() {
            DnsFree(result.query_records, DNS_FREE_RECORD_LIST);
        }
        Ok(ips)
    }
}

fn dns_query_ex_a_async(
    name: &str,
    cancel_requested: bool,
    copy_cancel: bool,
    reenter: bool,
    inspect_pending: bool,
) -> AsyncDnsOutcome {
    const DNS_TYPE_A: u16 = 1;
    const DNS_QUERY_REQUEST_VERSION1: u32 = 1;
    const DNS_QUERY_RESULTS_VERSION1: u32 = 1;
    const DNS_REQUEST_PENDING: i32 = 9506;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;

    let event = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) };
    if event.is_null() {
        return AsyncDnsOutcome {
            return_status: -1,
            initial_query_status: -1,
            callback_status: -1,
            callback_cancel_status: -1,
            callback_count: 0,
            cancel_status: None,
            cancel_copied: false,
            wait_status: WAIT_TIMEOUT,
            addresses: Vec::new(),
            reentry_return_status: None,
            reentry_initial_query_status: None,
            reentry_query_status: None,
            reentry_callback_count: None,
            stale_cancel_status: None,
            reentry_wait_status: None,
            reentry_addresses: None,
        };
    }

    let reentry_event = if reenter {
        unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) }
    } else {
        std::ptr::null_mut()
    };
    if reenter && reentry_event.is_null() {
        unsafe {
            CloseHandle(event);
        }
        return AsyncDnsOutcome {
            return_status: -1,
            initial_query_status: -1,
            callback_status: -1,
            callback_cancel_status: -1,
            callback_count: 0,
            cancel_status: None,
            cancel_copied: false,
            wait_status: WAIT_TIMEOUT,
            addresses: Vec::new(),
            reentry_return_status: None,
            reentry_initial_query_status: None,
            reentry_query_status: None,
            reentry_callback_count: None,
            stale_cancel_status: None,
            reentry_wait_status: None,
            reentry_addresses: None,
        };
    }

    let query_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let state = Box::new(AsyncDnsState {
        event,
        reentry_event,
        query_name,
        cancel: DnsQueryCancel { reserved: [0; 32] },
        result: DnsQueryResult {
            version: DNS_QUERY_RESULTS_VERSION1,
            query_status: 0,
            query_options: 0,
            query_records: std::ptr::null_mut(),
            reserved: std::ptr::null_mut(),
        },
        query_status: AtomicI32::new(-1),
        callback_cancel_status: AtomicI32::new(-1),
        callback_count: AtomicU32::new(0),
        addresses: Mutex::new(Vec::new()),
        reenter,
        inspect_pending,
        reentry_result: DnsQueryResult {
            version: DNS_QUERY_RESULTS_VERSION1,
            query_status: 0,
            query_options: 0,
            query_records: std::ptr::null_mut(),
            reserved: std::ptr::null_mut(),
        },
        reentry_started: AtomicU32::new(0),
        reentry_return_status: AtomicI32::new(-1),
        reentry_initial_query_status: AtomicI32::new(-1),
        reentry_query_status: AtomicI32::new(-1),
        reentry_callback_count: AtomicU32::new(0),
        stale_cancel_status: AtomicI32::new(-1),
        reentry_addresses: Mutex::new(Vec::new()),
    });
    let state_ptr = Box::into_raw(state);
    let request = DnsQueryRequest {
        version: DNS_QUERY_REQUEST_VERSION1,
        query_name: unsafe { (*state_ptr).query_name.as_ptr() },
        query_type: DNS_TYPE_A,
        query_options: 0,
        dns_server_list: std::ptr::null_mut(),
        interface_index: 0,
        completion: Some(dns_query_ex_completion),
        query_context: state_ptr as *mut c_void,
    };
    let return_status = unsafe {
        DnsQueryEx(
            &request,
            std::ptr::addr_of_mut!((*state_ptr).result),
            std::ptr::addr_of_mut!((*state_ptr).cancel),
        )
    };
    // Keep the API's actual result-field observation. In particular, a fast
    // worker may complete between the return and this read.
    let initial_query_status = if inspect_pending {
        unsafe {
            AtomicI32::from_ptr(std::ptr::addr_of_mut!((*state_ptr).result.query_status))
                .load(Ordering::Acquire)
        }
    } else {
        -1
    };
    let mut cancel_status = None;
    let mut wait_status = WAIT_OBJECT_0;
    let mut reentry_wait_status = None;

    unsafe {
        if return_status == DNS_REQUEST_PENDING {
            if cancel_requested {
                // Cancellation is a request; the callback remains the sole
                // completion/lifetime boundary and is always awaited below.
                // Copying the opaque handle verifies that cancellation uses
                // the generation token rather than the storage address.
                let copied_cancel = (*state_ptr).cancel;
                cancel_status = Some(if copy_cancel {
                    DnsCancelQuery(&copied_cancel)
                } else {
                    DnsCancelQuery(&(*state_ptr).cancel)
                });
            }
            wait_status = WaitForSingleObject(event, 5_000);
            if wait_status == WAIT_TIMEOUT {
                // Keep the state alive until the Runtime/provider acknowledges
                // the cancellation. This path should be rare, but avoids
                // freeing pQueryContext while a completion callback can arrive.
                if cancel_requested {
                    let copied_cancel = (*state_ptr).cancel;
                    cancel_status.get_or_insert(if copy_cancel {
                        DnsCancelQuery(&copied_cancel)
                    } else {
                        DnsCancelQuery(&(*state_ptr).cancel)
                    });
                }
                wait_status = WaitForSingleObject(event, 5_000);
            }
        } else if (*state_ptr).callback_count.load(Ordering::Acquire) == 0 {
            // DnsQueryEx documents that inline completion does not invoke the
            // callback. Consume and free the synchronous record list here.
            let status = consume_dns_result(
                std::ptr::addr_of_mut!((*state_ptr).result),
                &(*state_ptr).addresses,
            );
            (*state_ptr).query_status.store(status, Ordering::Release);
        }

        if reenter
            && wait_status == WAIT_OBJECT_0
            && (*state_ptr).reentry_started.load(Ordering::Acquire) != 0
        {
            reentry_wait_status = Some(WaitForSingleObject(reentry_event, 5_000));
        }

        let callback_count = (*state_ptr).callback_count.load(Ordering::Acquire);
        let callback_status = (*state_ptr).query_status.load(Ordering::Acquire);
        let addresses = (*state_ptr).addresses.lock().unwrap().clone();
        let reentry_started = (*state_ptr).reentry_started.load(Ordering::Acquire) != 0;
        let reentry_return_status = if reenter && reentry_started {
            Some((*state_ptr).reentry_return_status.load(Ordering::Acquire))
        } else {
            None
        };
        let reentry_initial_query_status = if reenter && reentry_started {
            Some(
                (*state_ptr)
                    .reentry_initial_query_status
                    .load(Ordering::Acquire),
            )
        } else {
            None
        };
        let reentry_query_status = if reenter && reentry_started {
            Some((*state_ptr).reentry_query_status.load(Ordering::Acquire))
        } else {
            None
        };
        let reentry_callback_count = if reenter && reentry_started {
            Some((*state_ptr).reentry_callback_count.load(Ordering::Acquire))
        } else {
            None
        };
        let reentry_addresses = if reenter && reentry_started {
            Some((*state_ptr).reentry_addresses.lock().unwrap().clone())
        } else {
            None
        };
        // The event is signaled from inside the callback, before that
        // callback returns. The waiting thread can therefore wake while the
        // callback still has a few instructions left to execute. Keep this
        // short-lived Probe allocation and both handles until process exit so
        // that callback lifetime does not depend on the event wake-up race.
        AsyncDnsOutcome {
            return_status,
            initial_query_status,
            callback_status,
            callback_cancel_status: (*state_ptr).callback_cancel_status.load(Ordering::Acquire),
            callback_count,
            cancel_status,
            cancel_copied: cancel_requested && copy_cancel,
            wait_status,
            addresses,
            reentry_return_status,
            reentry_initial_query_status,
            reentry_query_status,
            reentry_callback_count,
            stale_cancel_status: if reenter && reentry_started {
                Some((*state_ptr).stale_cancel_status.load(Ordering::Acquire))
            } else {
                None
            },
            reentry_wait_status,
            reentry_addresses,
        }
    }
}

fn print_parent() {
    print_runtime_marker();
    let snapshot = collect_host_snapshot();
    println!("=== PARENT PROBE ===");
    print!("{}", snapshot.render());
}

/// Stable smoke marker when envbox-runtime is injected into this process (ticket 04).
fn print_runtime_marker() {
    if runtime_loaded() {
        println!("EnvBox Runtime Loaded");
    }
}

fn runtime_loaded() -> bool {
    // Require the module to be mapped; env alone can false-positive from the Host.
    #[cfg(windows)]
    {
        use windows::core::w;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        unsafe {
            return GetModuleHandleW(w!("envbox-runtime64.dll")).is_ok()
                || GetModuleHandleW(w!("envbox-runtime32.dll")).is_ok();
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn print_child() {
    print!("{}", child_output());
}

fn child_output() -> String {
    let mut text = String::new();
    if runtime_loaded() {
        text.push_str("EnvBox Runtime Loaded\n");
    }
    text.push_str("=== CHILD PROBE ===\n");
    text.push_str(&collect_host_snapshot().render());
    text
}

fn spawn_child_probe() {
    let exe = std::env::current_exe().expect("current_exe");
    let output = Command::new(exe)
        .arg("--child")
        .output()
        .expect("failed to spawn child probe");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
}
