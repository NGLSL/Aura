//! envbox-probe: Host / Profile environment snapshot for EnvBox acceptance.

use envbox_probe::collect_host_snapshot;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let spawn_child = args.iter().any(|a| a == "--spawn-child");
    let is_child = args.iter().any(|a| a == "--child");

    // Optional DNS routing probe: `--resolve <name>` prints getaddrinfo results.
    // Optional DnsQuery_A smoke: `--resolve-dnsquery <name>`.
    let mut resolve_name: Option<String> = None;
    let mut resolve_dnsquery: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--resolve" && i + 1 < args.len() {
            resolve_name = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--resolve-dnsquery" && i + 1 < args.len() {
            resolve_dnsquery = Some(args[i + 1].clone());
            i += 2;
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

    print_parent();
    if spawn_child {
        spawn_child_probe();
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
        Err(_) => println!("<error>"),
    }
    println!();
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
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::core::w;
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
    print_runtime_marker();
    let snapshot = collect_host_snapshot();
    println!("=== CHILD PROBE ===");
    print!("{}", snapshot.render());
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
