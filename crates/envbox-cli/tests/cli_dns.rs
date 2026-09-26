//! DNS routing acceptance (tickets 24-26 + review): VirtualView routes
//! getaddrinfo and DnsQueryEx via Profile servers; Host mode leaves resolution
//! untouched; Fail Open never hangs; truncated / CNAME-only never fake NXDOMAIN.

use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Only one fixture DNS server may bind 127.0.0.1:53 at a time.
static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn envbox_with_root(root: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root);
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        cmd.env("ENVBOX_RUNTIME_DLL", dll);
    }
    apply_dns_port(&mut cmd);
    cmd
}

/// Test seam: host DNS proxies often own :53. Fixture binds a high port and
/// ENVBOX_DNS_UDP_PORT points the runtime client at it.
fn fixture_dns_port() -> u16 {
    std::env::var("ENVBOX_TEST_DNS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15353)
}

fn apply_dns_port(cmd: &mut Command) {
    cmd.env("ENVBOX_DNS_UDP_PORT", fixture_dns_port().to_string());
}

fn test_runtime_dll() -> Option<std::path::PathBuf> {
    if let Ok(dll) = std::env::var("ENVBOX_TEST_RUNTIME_DLL") {
        let p = std::path::PathBuf::from(dll);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("envbox-runtime64.dll"));
            }
            candidates.push(dir.join("envbox-runtime64.dll"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn probe_exe() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::path::PathBuf::from(env!("CARGO_BIN_EXE_envbox")).parent() {
        candidates.push(dir.join("envbox-probe.exe"));
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join("envbox-probe.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Value line following `Name:` in Probe text format.
fn field_after(text: &str, key: &str) -> String {
    let idx = text
        .find(key)
        .unwrap_or_else(|| panic!("missing key {key} in:\n{text}"));
    let rest = &text[idx + key.len()..];
    rest.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn make_profile(root: &std::path::Path, dns_servers: &[&str], virtual_view: bool) -> String {
    let mut args = vec![
        "profile".to_string(),
        "add".to_string(),
        "--name".to_string(),
        "DNS Route".to_string(),
        "--locale".to_string(),
        "en-US".to_string(),
        "--ui-language".to_string(),
        "en-US".to_string(),
        "--region".to_string(),
        "US".to_string(),
        "--tz-windows".to_string(),
        "Pacific Standard Time".to_string(),
        "--tz-iana".to_string(),
        "America/Los_Angeles".to_string(),
    ];
    if virtual_view {
        args.push("--dns-mode".into());
        args.push("virtual_view".into());
    }
    for s in dns_servers {
        args.push("--dns".into());
        args.push((*s).to_string());
    }
    let out = envbox_with_root(root).args(&args).output().expect("profile add");
    assert!(out.status.success(), "profile add failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_probe_resolve(
    root: &std::path::Path,
    dll: &std::path::Path,
    profile_id: &str,
    name: &str,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .args(["run", "--profile", profile_id])
        .arg(probe_exe().expect("envbox-probe.exe required"))
        .args(["--resolve", name]);
    apply_dns_port(&mut cmd);
    cmd.output().expect("run probe --resolve")
}

fn run_probe_dnsquery(
    root: &std::path::Path,
    dll: &std::path::Path,
    profile_id: &str,
    name: &str,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .args(["run", "--profile", profile_id])
        .arg(probe_exe().expect("envbox-probe.exe required"))
        .args(["--resolve-dnsquery", name]);
    apply_dns_port(&mut cmd);
    cmd.output().expect("run probe --resolve-dnsquery")
}

fn run_probe_dnsquery_ex(
    root: &std::path::Path,
    dll: &std::path::Path,
    profile_id: &str,
    name: &str,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .args(["run", "--profile", profile_id, "--audit"])
        .arg(probe_exe().expect("envbox-probe.exe required"))
        .args(["--resolve-dnsquery-ex", name]);
    apply_dns_port(&mut cmd);
    cmd.output().expect("run probe --resolve-dnsquery-ex")
}

// --- fixture DNS server (UDP 127.0.0.1:53) ---

const FIXTURE_NAME: &str = "fixture.test";
const FIXTURE_A: [u8; 4] = [10, 99, 0, 1];
const CNAME_A_NAME: &str = "cname-a.test";
const CNAME_B_NAME: &str = "cname-b.test";
const DANGLING_NAME: &str = "dangling.invalid";
const NODATA_NAME: &str = "nodata.test";
const REFERRAL_NAME: &str = "referral.test";
const EAI_NONAME_STATUS: &str = "<error 11001>";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FixtureMode {
    /// fixture.test -> A 10.99.0.1; others NXDOMAIN.
    Address,
    /// Every response has TC=1 (unusable / truncated).
    Truncated,
    /// cname-only.test -> CNAME dangling.invalid (NOERROR, no A). Chain never
    /// yields an address.
    CnameOnly,
    /// cname-a.test -> cname-b.test -> fixture.test -> A.
    CnameChain,
    /// NOERROR + empty answer + SOA authority (RFC 2308 NODATA).
    Nodata,
    /// NOERROR + empty answer + NS authority (referral; no SOA).
    Referral,
}

/// Bind fixture DNS on the test port (not 53: host DNS proxies own :53).
fn try_bind_fixture_dns() -> Option<UdpSocket> {
    let port = fixture_dns_port();
    match UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
        Ok(sock) => {
            let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
            Some(sock)
        }
        Err(e) => {
            eprintln!("skip: cannot bind 127.0.0.1:{port} ({e}); DNS fixture unavailable");
            None
        }
    }
}

fn skip_name(buf: &[u8], off: &mut usize) -> bool {
    let mut guard = 0;
    while *off < buf.len() {
        let c = buf[*off];
        if c == 0 {
            *off += 1;
            return true;
        }
        if c & 0xC0 == 0xC0 {
            if *off + 2 > buf.len() {
                return false;
            }
            *off += 2;
            return true;
        }
        if c & 0xC0 != 0 {
            return false;
        }
        *off += 1 + c as usize;
        guard += 1;
        if guard > 128 {
            return false;
        }
    }
    false
}

fn read_name_labels(buf: &[u8], mut off: usize) -> Option<String> {
    let mut labels = Vec::new();
    let mut guard = 0;
    while off < buf.len() {
        let c = buf[off];
        if c == 0 {
            break;
        }
        if c & 0xC0 == 0xC0 {
            return None;
        }
        if c & 0xC0 != 0 {
            return None;
        }
        off += 1;
        if off + c as usize > buf.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&buf[off..off + c as usize]).to_ascii_lowercase());
        off += c as usize;
        guard += 1;
        if guard > 64 {
            return None;
        }
    }
    Some(labels.join("."))
}

fn encode_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

fn build_fixture_response(query: &[u8], mode: FixtureMode) -> Vec<u8> {
    if query.len() < 12 {
        return Vec::new();
    }
    let id = &query[0..2];
    let qd = u16::from_be_bytes([query[4], query[5]]);
    let mut off = 12usize;
    let qname = read_name_labels(query, off);
    if !skip_name(query, &mut off) || off + 4 > query.len() {
        return Vec::new();
    }
    let qtype = u16::from_be_bytes([query[off], query[off + 1]]);
    off += 4;
    let question = query[12..off].to_vec();
    let qname = qname.unwrap_or_default();

    // Truncated: unusable answer, never a final result.
    if mode == FixtureMode::Truncated {
        let mut resp = Vec::with_capacity(64);
        resp.extend_from_slice(id);
        // QR + TC + rcode
        let flags: u16 = 0x8200;
        resp.extend_from_slice(&flags.to_be_bytes());
        resp.extend_from_slice(&qd.to_be_bytes());
        resp.extend_from_slice(&0u16.to_be_bytes());
        resp.extend_from_slice(&0u16.to_be_bytes());
        resp.extend_from_slice(&0u16.to_be_bytes());
        resp.extend_from_slice(&question);
        return resp;
    }

    enum Answer {
        None,
        A([u8; 4]),
        Cname(&'static str),
    }

    let answer = match mode {
        FixtureMode::Address => {
            if qname == FIXTURE_NAME && qtype == 1 {
                Answer::A(FIXTURE_A)
            } else {
                Answer::None
            }
        }
        FixtureMode::CnameOnly => {
            // NOERROR + CNAME only (no A/AAAA) for the probe name.
            if qtype == 1 || qtype == 28 {
                Answer::Cname(DANGLING_NAME)
            } else {
                Answer::None
            }
        }
        FixtureMode::CnameChain => match qname.as_str() {
            n if n == CNAME_A_NAME => Answer::Cname(CNAME_B_NAME),
            n if n == CNAME_B_NAME => Answer::Cname(FIXTURE_NAME),
            n if n == FIXTURE_NAME && qtype == 1 => Answer::A(FIXTURE_A),
            _ => Answer::None,
        },
        FixtureMode::Nodata | FixtureMode::Referral | FixtureMode::Truncated => {
            Answer::None
        }
    };

    let soa_authority = mode == FixtureMode::Nodata && qname == NODATA_NAME;
    let referral_authority = mode == FixtureMode::Referral && qname == REFERRAL_NAME;

    // NXDOMAIN for Address/other miss; NOERROR empty for CnameOnly dangling.
    let (rcode, ancount): (u8, u16) = match &answer {
        Answer::A(_) | Answer::Cname(_) => (0, 1),
        Answer::None => {
            if soa_authority || referral_authority {
                (0, 0)
            } else if mode == FixtureMode::CnameOnly || mode == FixtureMode::CnameChain {
                (0, 0)
            } else if qname == FIXTURE_NAME || qname.ends_with(".test") {
                (0, 0)
            } else {
                (3, 0)
            }
        }
    };

    let mut resp = Vec::with_capacity(128);
    resp.extend_from_slice(id);
    // QR + AA + RA + rcode
    let flags: u16 = 0x8480 | rcode as u16;
    resp.extend_from_slice(&flags.to_be_bytes());
    resp.extend_from_slice(&qd.to_be_bytes());
    resp.extend_from_slice(&ancount.to_be_bytes());
    let nscount: u16 = if soa_authority || referral_authority { 1 } else { 0 };
    resp.extend_from_slice(&nscount.to_be_bytes());
    resp.extend_from_slice(&0u16.to_be_bytes());
    resp.extend_from_slice(&question);
    if ancount == 1 {
        resp.extend_from_slice(&[0xC0, 0x0C]);
        match &answer {
            Answer::A(ip) => {
                resp.extend_from_slice(&1u16.to_be_bytes()); // A
                resp.extend_from_slice(&1u16.to_be_bytes()); // IN
                resp.extend_from_slice(&60u32.to_be_bytes());
                resp.extend_from_slice(&4u16.to_be_bytes());
                resp.extend_from_slice(ip);
            }
            Answer::Cname(target) => {
                let rdata = encode_name(target);
                resp.extend_from_slice(&5u16.to_be_bytes()); // CNAME
                resp.extend_from_slice(&1u16.to_be_bytes()); // IN
                resp.extend_from_slice(&60u32.to_be_bytes());
                resp.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
                resp.extend_from_slice(&rdata);
            }
            Answer::None => {}
        }
    }
    if soa_authority {
        // Authority SOA marks an authoritative NODATA response. Keep the
        // names uncompressed so the fixture exercises the parser's normal
        // authority-section name skipping and does not rely on answer data.
        resp.extend_from_slice(&[0xC0, 0x0C]);
        resp.extend_from_slice(&6u16.to_be_bytes()); // SOA
        resp.extend_from_slice(&1u16.to_be_bytes()); // IN
        resp.extend_from_slice(&60u32.to_be_bytes());
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&encode_name("ns1.nodata.test"));
        rdata.extend_from_slice(&encode_name("hostmaster.nodata.test"));
        rdata.extend_from_slice(&1u32.to_be_bytes()); // serial
        rdata.extend_from_slice(&3600u32.to_be_bytes()); // refresh
        rdata.extend_from_slice(&600u32.to_be_bytes()); // retry
        rdata.extend_from_slice(&86400u32.to_be_bytes()); // expire
        rdata.extend_from_slice(&60u32.to_be_bytes()); // minimum
        resp.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        resp.extend_from_slice(&rdata);
    } else if referral_authority {
        // An NS-only authority section is a referral, not RFC 2308 NODATA.
        resp.extend_from_slice(&[0xC0, 0x0C]);
        resp.extend_from_slice(&2u16.to_be_bytes()); // NS
        resp.extend_from_slice(&1u16.to_be_bytes()); // IN
        resp.extend_from_slice(&60u32.to_be_bytes());
        let rdata = encode_name("ns1.referral.test");
        resp.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        resp.extend_from_slice(&rdata);
    }
    resp
}

fn spawn_fixture_dns(sock: UdpSocket, mode: FixtureMode) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while !stop2.load(Ordering::SeqCst) {
            match sock.recv_from(&mut buf) {
                Ok((n, peer)) => {
                    let resp = build_fixture_response(&buf[..n], mode);
                    if !resp.is_empty() {
                        let _ = sock.send_to(&resp, peer);
                    }
                }
                Err(_) => continue,
            }
        }
    });
    stop
}

fn lock_fixture() -> MutexGuard<'static, ()> {
    FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Ticket 26.1+2+3: VirtualView + fixture DNS resolves fixture.test to 10.99.0.1.
#[test]
fn virtual_view_resolves_via_profile_dns_fixture() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_resolve(&root, &dll, &profile_id, FIXTURE_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "getaddrinfo:");
    assert_eq!(
        got, "10.99.0.1",
        "VirtualView must route fixture.test via Profile DNS:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 26.4: Host mode must not force the fixture answer.
#[test]
fn host_mode_does_not_force_fixture_result() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    // Default DnsMode::Host, no profile DNS servers.
    let profile_id = make_profile(&root, &[], false);

    let out = run_probe_resolve(&root, &dll, &profile_id, FIXTURE_NAME);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "getaddrinfo:");
    assert_ne!(
        got, "10.99.0.1",
        "Host mode must not force fixture DNS result:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 26.5: unreachable Profile server times out (process exits, never hangs).
#[test]
fn unreachable_dns_server_does_not_hang() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    // 192.0.2.1 is TEST-NET-1 (documentation); no DNS listener expected.
    let profile_id = make_profile(&root, &["192.0.2.1"], true);

    let started = Instant::now();
    let out = run_probe_resolve(&root, &dll, &profile_id, FIXTURE_NAME);
    let elapsed = started.elapsed();
    assert!(out.status.success(), "run failed: {out:?}");
    assert!(
        elapsed < Duration::from_secs(20),
        "DNS routing must be time-bounded, took {elapsed:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Fail Open or definitive miss: must not return the fixture address.
    let got = field_after(&stdout, "getaddrinfo:");
    assert_ne!(got, "10.99.0.1", "must not invent fixture answer:\n{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 26.6: Host DNS / other processes unchanged after DNS routing runs.
#[test]
fn host_dns_unchanged_after_dns_routing() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        // Still assert localhost works on Host.
        let addrs: Vec<_> = ("localhost", 0u16)
            .to_socket_addrs()
            .expect("host resolve localhost")
            .collect();
        assert!(!addrs.is_empty());
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let _ = run_probe_resolve(&root, &dll, &profile_id, FIXTURE_NAME);
    stop.store(true, Ordering::SeqCst);

    // This test process is not injected: Host resolution must still work.
    let addrs: Vec<_> = ("localhost", 0u16)
        .to_socket_addrs()
        .expect("host resolve localhost after DNS routing")
        .map(|a| a.ip().to_string())
        .collect();
    assert!(
        !addrs.is_empty(),
        "Host must still resolve localhost after DNS routing run"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Ticket 26.4 contrast: same fixture under VirtualView vs Host differs.
#[test]
fn virtual_view_and_host_resolution_contrast() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virt = make_profile(&root, &["127.0.0.1"], true);
    let host = make_profile(&root, &[], false);

    let virt_out = run_probe_resolve(&root, &dll, &virt, FIXTURE_NAME);
    let host_out = run_probe_resolve(&root, &dll, &host, FIXTURE_NAME);
    stop.store(true, Ordering::SeqCst);

    let v = field_after(&String::from_utf8_lossy(&virt_out.stdout), "getaddrinfo:");
    let h = field_after(&String::from_utf8_lossy(&host_out.stdout), "getaddrinfo:");
    assert_eq!(v, "10.99.0.1", "virtual: {v}");
    assert_ne!(v, h, "VirtualView vs Host must contrast for fixture.test");
    let _ = std::fs::remove_dir_all(&root);
}

/// Sorted unique IP list from a probe getaddrinfo/DnsQuery value line.
fn parse_ip_set(value: &str) -> Vec<String> {
    let mut ips: Vec<String> = value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "<error>" && s != "<empty>" && !s.starts_with("<error"))
        .collect();
    ips.sort();
    ips.dedup();
    ips
}

/// Review Hard: TC=1 is unusable. VirtualView must Fail Open to Host resolve
/// (same result as Host profile), never invent NXDOMAIN/empty as final.
#[test]
fn truncated_response_fails_open_like_host() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Truncated);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virt = make_profile(&root, &["127.0.0.1"], true);
    let host = make_profile(&root, &[], false);

    // localhost must resolve on Host; TC fixture forces Fail Open to same path.
    let virt_out = run_probe_resolve(&root, &dll, &virt, "localhost");
    let host_out = run_probe_resolve(&root, &dll, &host, "localhost");
    stop.store(true, Ordering::SeqCst);

    assert!(virt_out.status.success(), "virt run: {virt_out:?}");
    assert!(host_out.status.success(), "host run: {host_out:?}");
    let v = field_after(&String::from_utf8_lossy(&virt_out.stdout), "getaddrinfo:");
    let h = field_after(&String::from_utf8_lossy(&host_out.stdout), "getaddrinfo:");
    assert_ne!(v, "<error>", "TC must Fail Open, not error out:\n{v}");
    assert_eq!(
        parse_ip_set(&v),
        parse_ip_set(&h),
        "TC truncated must Fail Open to Host resolve (virt={v} host={h})"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Review Worst: CNAME-only (NOERROR, no A/AAAA) must not fake NXDOMAIN.
/// Follow CNAME; if still no address, Fail Open -- match Host resolve.
#[test]
fn cname_only_fails_open_like_host() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::CnameOnly);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virt = make_profile(&root, &["127.0.0.1"], true);
    let host = make_profile(&root, &[], false);

    let virt_out = run_probe_resolve(&root, &dll, &virt, "localhost");
    let host_out = run_probe_resolve(&root, &dll, &host, "localhost");
    stop.store(true, Ordering::SeqCst);

    assert!(virt_out.status.success(), "virt run: {virt_out:?}");
    assert!(host_out.status.success(), "host run: {host_out:?}");
    let v = field_after(&String::from_utf8_lossy(&virt_out.stdout), "getaddrinfo:");
    let h = field_after(&String::from_utf8_lossy(&host_out.stdout), "getaddrinfo:");
    // Must not return fabricated empty/NXDOMAIN for a name Host can resolve.
    assert_ne!(v, "<error>", "CNAME-only must Fail Open, not fake NXDOMAIN:\n{v}");
    assert_ne!(v, "<empty>", "CNAME-only must Fail Open, not empty:\n{v}");
    assert_eq!(
        parse_ip_set(&v),
        parse_ip_set(&h),
        "CNAME-only must Fail Open to Host resolve (virt={v} host={h})"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Review: CNAME chain is followed to the A record (max hops).
#[test]
fn cname_chain_resolves_target_address() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::CnameChain);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_resolve(&root, &dll, &profile_id, CNAME_A_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "getaddrinfo:");
    assert_eq!(
        got, "10.99.0.1",
        "CNAME chain cname-a -> cname-b -> fixture.test must resolve A:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Review: unreachable Profile server Fail Open outcome matches Host resolve.
#[test]
fn unreachable_matches_host_outcome() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virt = make_profile(&root, &["192.0.2.1"], true);
    let host = make_profile(&root, &[], false);

    let virt_out = run_probe_resolve(&root, &dll, &virt, "localhost");
    let host_out = run_probe_resolve(&root, &dll, &host, "localhost");
    assert!(virt_out.status.success(), "virt run: {virt_out:?}");
    assert!(host_out.status.success(), "host run: {host_out:?}");
    let v = field_after(&String::from_utf8_lossy(&virt_out.stdout), "getaddrinfo:");
    let h = field_after(&String::from_utf8_lossy(&host_out.stdout), "getaddrinfo:");
    assert_eq!(
        parse_ip_set(&v),
        parse_ip_set(&h),
        "unreachable Profile DNS must Fail Open to Host outcome (virt={v} host={h})"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Optional smoke: DnsQuery_A routes via Profile DNS under VirtualView.
#[test]
fn dnsquery_a_smoke_routes_fixture() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_dnsquery(&root, &dll, &profile_id, FIXTURE_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "DnsQuery_A:");
    assert_eq!(
        got, "10.99.0.1",
        "DnsQuery_A must route fixture.test via Profile DNS:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// DnsQueryEx must route synchronous queries through the Profile DNS server
/// under VirtualView instead of silently using the Host resolver.
#[test]
fn dnsquery_ex_smoke_routes_fixture() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_dnsquery_ex(&root, &dll, &profile_id, FIXTURE_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "DnsQueryEx_A:");
    let audit = std::fs::read_dir(root.join("audit"))
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        got, "10.99.0.1",
        "DnsQueryEx must route fixture.test via Profile DNS:\n{stdout}\naudit:\n{audit}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A mixed IPv4/IPv6 Profile still uses the bounded Profile wire route. The
/// DnsQueryEx hook must not require a single-family DNS_ADDR_ARRAY before it
/// can route a synchronous A lookup.
#[test]
fn dnsquery_ex_mixed_profile_routes_fixture() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1", "::1"], true);

    let out = run_probe_dnsquery_ex(&root, &dll, &profile_id, FIXTURE_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "DnsQueryEx_A:");
    assert_eq!(
        got, "10.99.0.1",
        "mixed IPv4/IPv6 Profile must route fixture.test via Profile DNS:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// RFC 2308 NODATA must remain a definitive negative answer instead of
/// falling through to the Host resolver.
#[test]
fn dnsquery_ex_authoritative_nodata_returns_no_records() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Nodata);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_dnsquery_ex(&root, &dll, &profile_id, NODATA_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "DnsQueryEx_A:");
    assert_eq!(
        got, "<error 9501>",
        "SOA authority NODATA must return DNS_INFO_NO_RECORDS:
{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// RFC 2308 NODATA must map to EAI_NONAME for getaddrinfo instead of falling
/// through to the Host resolver. Windows exposes EAI_NONAME as 11001.
#[test]
fn getaddrinfo_authoritative_nodata_returns_eai_noname() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Nodata);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_resolve(&root, &dll, &profile_id, NODATA_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "getaddrinfo:");
    assert_eq!(
        got, EAI_NONAME_STATUS,
        "SOA authority NODATA must return EAI_NONAME, not Host data:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// An NS-only authority response is a referral. It has no definitive
/// negative answer, so the route must retain the existing Fail Open behavior.
#[test]
fn dnsquery_ex_referral_fails_open() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Referral);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virtual_id = make_profile(&root, &["127.0.0.1"], true);
    let host_id = make_profile(&root, &[], false);

    let virtual_out = run_probe_dnsquery_ex(&root, &dll, &virtual_id, REFERRAL_NAME);
    let host_out = run_probe_dnsquery_ex(&root, &dll, &host_id, REFERRAL_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(virtual_out.status.success(), "virtual run failed: {virtual_out:?}");
    assert!(host_out.status.success(), "host run failed: {host_out:?}");
    let virtual_value = field_after(
        &String::from_utf8_lossy(&virtual_out.stdout),
        "DnsQueryEx_A:",
    );
    let host_value = field_after(&String::from_utf8_lossy(&host_out.stdout), "DnsQueryEx_A:");
    assert_eq!(
        virtual_value, host_value,
        "NS-only referral must Fail Open to the Host resolver (virtual={virtual_value} host={host_value})"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// An NS-only authority response is a referral for getaddrinfo as well. It
/// must retain the existing Fail Open behavior and match the Host profile.
#[test]
fn getaddrinfo_referral_fails_open() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Referral);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virtual_id = make_profile(&root, &["127.0.0.1"], true);
    let host_id = make_profile(&root, &[], false);

    let virtual_out = run_probe_resolve(&root, &dll, &virtual_id, REFERRAL_NAME);
    let host_out = run_probe_resolve(&root, &dll, &host_id, REFERRAL_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(virtual_out.status.success(), "virtual run failed: {virtual_out:?}");
    assert!(host_out.status.success(), "host run failed: {host_out:?}");
    let virtual_value = field_after(
        &String::from_utf8_lossy(&virtual_out.stdout),
        "getaddrinfo:",
    );
    let host_value = field_after(&String::from_utf8_lossy(&host_out.stdout), "getaddrinfo:");
    assert_eq!(
        virtual_value, host_value,
        "NS-only referral must Fail Open to the Host resolver (virtual={virtual_value} host={host_value})"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Optional smoke: DnsQuery_A under TC must Fail Open (not invent NXDOMAIN).
#[test]
fn dnsquery_a_truncated_fails_open() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::Truncated);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virt = make_profile(&root, &["127.0.0.1"], true);
    let host = make_profile(&root, &[], false);

    let virt_out = run_probe_dnsquery(&root, &dll, &virt, "localhost");
    let host_out = run_probe_dnsquery(&root, &dll, &host, "localhost");
    stop.store(true, Ordering::SeqCst);
    assert!(virt_out.status.success(), "virt run: {virt_out:?}");
    assert!(host_out.status.success(), "host run: {host_out:?}");
    let v = field_after(&String::from_utf8_lossy(&virt_out.stdout), "DnsQuery_A:");
    let h = field_after(&String::from_utf8_lossy(&host_out.stdout), "DnsQuery_A:");
    assert_eq!(
        parse_ip_set(&v),
        parse_ip_set(&h),
        "DnsQuery_A TC must Fail Open to Host (virt={v} host={h})"
    );
    let _ = std::fs::remove_dir_all(&root);
}
