//! DNS routing acceptance (tickets 24-26 + review): VirtualView routes
//! getaddrinfo and DnsQueryEx via Profile servers; Host mode leaves resolution
//! untouched; Profile failures never query Host; incomplete results never fake NXDOMAIN.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, ToSocketAddrs, UdpSocket};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Fixture tests share one explicitly configured loopback port.
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

/// Typed Profile upstreams explicitly select the fixture's high port.
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
    if let Some(path) = std::env::var_os("ENVBOX_TEST_PROBE_EXE") {
        let path = std::path::PathBuf::from(path);
        assert!(
            path.is_file(),
            "configured Probe missing: {}",
            path.display()
        );
        return Some(path);
    }
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
    args.extend(["--dns-mode".into(), "host".into()]);
    let out = envbox_with_root(root)
        .args(&args)
        .output()
        .expect("profile add");
    assert!(out.status.success(), "profile add failed: {out:?}");
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if virtual_view {
        for server in dns_servers {
            let out = envbox_with_root(root)
                .args([
                    "profile",
                    "dns",
                    "add",
                    &id,
                    "--type",
                    "udp",
                    "--address",
                    server,
                    "--port",
                    &fixture_dns_port().to_string(),
                ])
                .output()
                .expect("add typed fixture upstream");
            assert!(out.status.success(), "typed DNS add failed: {out:?}");
        }
        let out = envbox_with_root(root)
            .args([
                "profile",
                "dns",
                "set",
                &id,
                "--mode",
                "virtual_view",
                "--strict",
                "true",
            ])
            .output()
            .expect("set strict Profile DNS");
        assert!(out.status.success(), "strict DNS set failed: {out:?}");
    }
    id
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
        .args(["run", "--profile", profile_id])
        .arg(probe_exe().expect("envbox-probe.exe required"))
        .args(["--resolve-dnsquery-ex", name]);
    apply_dns_port(&mut cmd);
    cmd.output().expect("run probe --resolve-dnsquery-ex")
}

fn run_probe_dnsquery_ex_async(
    root: &std::path::Path,
    dll: &std::path::Path,
    profile_id: &str,
    name: &str,
    cancel: bool,
    copy_cancel: bool,
    reenter: bool,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .args(["run", "--profile", profile_id])
        .arg(probe_exe().expect("envbox-probe.exe required"))
        .args([
            "--resolve-dnsquery-ex-async",
            name,
            "--inspect-pending-status",
        ]);
    if cancel {
        cmd.arg("--cancel");
    }
    if copy_cancel {
        cmd.arg("--copy-cancel");
    }
    if reenter {
        cmd.arg("--reenter");
    }
    apply_dns_port(&mut cmd);
    cmd.output().expect("run probe --resolve-dnsquery-ex-async")
}

fn run_probe_dns_system_settings(
    root: &std::path::Path,
    dll: &std::path::Path,
    profile_id: &str,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_envbox"));
    cmd.env("ENVBOX_CONFIG_ROOT", root)
        .env("ENVBOX_RUNTIME_DLL", dll)
        .args(["run", "--profile", profile_id, "--audit"])
        .arg(probe_exe().expect("envbox-probe.exe required"))
        .arg("--dns-system-settings");
    apply_dns_port(&mut cmd);
    cmd.output().expect("run probe --dns-system-settings")
}

// --- fixture DNS server (UDP 127.0.0.1:<test port>) ---

const FIXTURE_NAME: &str = "fixture.test";
const ASYNC_FIXTURE_NAME: &str = "async-fixture.test";
const ASYNC_CANCEL_NAME: &str = "async-cancel.test";
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
    /// Address response after a delay, allowing an async cancellation request
    /// to reach the custom Runtime worker before it completes.
    DelayedAddress,
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
    Servfail,
    Refused,
    Nxdomain,
    Silent,
}

/// Bind fixture DNS on the test port (not 53: host DNS proxies own :53).
fn try_bind_fixture_dns() -> Option<UdpSocket> {
    try_bind_fixture_dns_port(fixture_dns_port())
}

/// Keep the explicit port helper for tests that need to exercise a different
/// fixture binding. The custom async Runtime path uses the same high-port
/// seam as the synchronous wire client.
fn try_bind_fixture_dns_port(port: u16) -> Option<UdpSocket> {
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

    if mode == FixtureMode::Silent {
        return Vec::new();
    }
    let rcode = match mode {
        FixtureMode::Servfail => Some(2),
        FixtureMode::Refused => Some(5),
        FixtureMode::Nxdomain => Some(3),
        _ => None,
    };
    if let Some(rcode) = rcode {
        let mut response = query[..off].to_vec();
        response[2..4].copy_from_slice(&(0x8480u16 | rcode).to_be_bytes());
        response[6..12].fill(0);
        return response;
    }

    if qname == "rr.servfail.test" {
        let mut response = query[..off].to_vec();
        response[2..4].copy_from_slice(&0x8482u16.to_be_bytes());
        response[6..12].fill(0);
        return response;
    }

    if mode != FixtureMode::Truncated
        && (qname == "rr.fixture.test"
            || qname == "rr.mismatch.test"
            || qname == "rr.wrongtype.test"
            || qname.is_empty()
            || (qname == "localhost" && qtype == 65))
    {
        let rdata = match qtype {
            1 => vec![10, 99, 0, 1],
            64 | 65 => vec![0, 1, 0], // priority 1, root target, no svcparams
            16 => {
                let mut bytes = vec![14];
                bytes.extend_from_slice(b"profile-marker");
                bytes
            }
            2 | 5 | 12 => encode_name("target.fixture.test"),
            33 => {
                let mut bytes = vec![0, 7, 0, 11, 1, 187];
                bytes.extend_from_slice(&encode_name("target.fixture.test"));
                bytes
            }
            65280 => vec![0xde, 0xad, 0xbe, 0xef],
            _ => vec![],
        };
        let mut response = query[..off].to_vec();
        response[2..4].copy_from_slice(&0x8480u16.to_be_bytes());
        response[6..8].copy_from_slice(&1u16.to_be_bytes());
        response[8..12].fill(0);
        response.extend_from_slice(&[0xc0, 0x0c]);
        response.extend_from_slice(&qtype.to_be_bytes());
        response.extend_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&60u32.to_be_bytes());
        response.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        response.extend_from_slice(&rdata);
        if qname == "rr.mismatch.test" {
            response[13] = b'x';
        }
        if qname == "rr.wrongtype.test" {
            response[off - 4..off - 2].copy_from_slice(&1u16.to_be_bytes());
        }
        return response;
    }

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
        FixtureMode::Address | FixtureMode::DelayedAddress => {
            if (qname == FIXTURE_NAME
                || qname == ASYNC_FIXTURE_NAME
                || qname == ASYNC_CANCEL_NAME
                || qname == "aura-dns-view")
                && qtype == 1
            {
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
        FixtureMode::Nodata
        | FixtureMode::Referral
        | FixtureMode::Truncated
        | FixtureMode::Servfail
        | FixtureMode::Refused
        | FixtureMode::Nxdomain
        | FixtureMode::Silent => Answer::None,
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
    let nscount: u16 = if soa_authority || referral_authority {
        1
    } else {
        0
    };
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

struct FixtureServer {
    stop: Arc<AtomicBool>,
    queries: Arc<AtomicU32>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct TcpFixture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<u32>>,
}

impl TcpFixture {
    fn finish(mut self) -> u32 {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .unwrap()
            .join()
            .expect("TCP fixture thread")
    }
}

impl Drop for TcpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // Preserve the original assertion panic while releasing the listener.
            let _ = thread.join();
        }
    }
}

impl std::ops::Deref for FixtureServer {
    type Target = AtomicBool;
    fn deref(&self) -> &AtomicBool {
        &self.stop
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("fixture DNS thread");
        }
    }
}

fn spawn_fixture_dns(sock: UdpSocket, mode: FixtureMode) -> FixtureServer {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let queries = Arc::new(AtomicU32::new(0));
    let seen = queries.clone();
    let thread = std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while !stop2.load(Ordering::SeqCst) {
            match sock.recv_from(&mut buf) {
                Ok((n, peer)) => {
                    seen.fetch_add(1, Ordering::SeqCst);
                    if mode == FixtureMode::DelayedAddress {
                        std::thread::sleep(Duration::from_millis(750));
                    }
                    let resp = build_fixture_response(&buf[..n], mode);
                    if !resp.is_empty() {
                        let _ = sock.send_to(&resp, peer);
                    }
                }
                Err(_) => continue,
            }
        }
    });
    FixtureServer {
        stop,
        queries,
        thread: Some(thread),
    }
}

fn lock_fixture() -> MutexGuard<'static, ()> {
    FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A virtual computer name must not become a Host resolver exemption.
#[test]
fn strict_virtual_computer_name_queries_profile_dns() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let server = spawn_fixture_dns(
        try_bind_fixture_dns().expect("fixture bind"),
        FixtureMode::Address,
    );
    let root = std::env::temp_dir().join(format!("envbox-dns-identity-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let name = "AURA-DNS-VIEW";
    let update = envbox_with_root(&root)
        .args([
            "profile",
            "identity",
            "set",
            &profile,
            "--computer-name",
            name,
        ])
        .output()
        .unwrap();
    assert!(update.status.success(), "{update:?}");
    for run in [run_probe_resolve, run_probe_dnsquery, run_probe_dnsquery_ex] {
        let before = server.queries.load(Ordering::SeqCst);
        let output = run(&root, &dll, &profile, name);
        assert!(output.status.success(), "{output:?}");
        assert!(
            server.queries.load(Ordering::SeqCst) > before,
            "virtual computer name bypassed Profile upstream: {output:?}"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("10.99.0.1"),
            "must return the fixture address rather than a Host result: {output:?}"
        );
    }
}

/// Unsupported native providers must reject synchronously before creating work.
/// Packet counts cover this Profile fixture only, not system-wide Host traffic.
#[test]
fn strict_unsupported_entrypoints_reject_without_pending_work() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let stop = spawn_fixture_dns(
        try_bind_fixture_dns().expect("fixture bind"),
        FixtureMode::Address,
    );
    let root = std::env::temp_dir().join(format!("envbox-dns-strict-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let host = make_profile(&root, &[], false);
    let probe = probe_exe().expect("probe required");
    let cases = [
        ("ex-a", "event", "10045"),
        ("ex-a", "callback", "10045"),
        ("ex-a", "namespace", "10045"),
        ("ex-a", "provider", "10045"),
        ("ex-a", "sync-flags", "10045"),
        ("ex-w", "flags", "10045"),
        ("ex-w", "sync-flags", "10045"),
        ("ex-w", "namespace", "10045"),
        ("ex-w", "provider", "10045"),
        ("gai-a", "sync-flags", "10045"),
        ("gai-w", "sync-flags", "10045"),
        ("raw", "name", "50"),
        ("raw", "packet", "50"),
        ("null-ex", "request", "87"),
        ("null-ex", "result", "87"),
    ];
    for (api, mode, expected) in cases {
        let run = |id: &str| {
            envbox_with_root(&root)
                .env("ENVBOX_RUNTIME_DLL", &dll)
                .args(["run", "--profile", id])
                .arg(&probe)
                .args(["--dns-strict", api, mode])
                .output()
                .expect("strict probe")
        };
        let out = run(&profile);
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{api}/{mode}: {out:?}");
        if api == "raw" && text.contains("StrictProbe_Available:\nfalse") {
            eprintln!("DnsQueryRaw export unavailable; {mode} not covered");
            continue;
        }
        assert_eq!(
            field_after(&text, "StrictProbe_Status:"),
            expected,
            "{api}/{mode}: {text}"
        );
        if api != "null-ex" {
            for key in [
                "StrictProbe_InitialCallbacks:",
                "StrictProbe_FinalCallbacks:",
            ] {
                assert_eq!(field_after(&text, key), "0", "{api}/{mode}: {text}");
            }
            for key in ["StrictProbe_InitialEvent:", "StrictProbe_FinalEvent:"] {
                assert_eq!(field_after(&text, key), "258", "{api}/{mode}: {text}");
            }
            assert_eq!(
                field_after(&text, "StrictProbe_InitialToken:"),
                "false",
                "{api}/{mode}: {text}"
            );
        }
        let control = Command::new(&probe)
            .args(["--dns-strict", api, mode])
            .output()
            .expect("native Host control");
        let host_out = run(&host);
        assert!(control.status.success() && host_out.status.success());
        assert_eq!(
            field_after(
                &String::from_utf8_lossy(&host_out.stdout),
                "StrictProbe_Status:"
            ),
            field_after(
                &String::from_utf8_lossy(&control.stdout),
                "StrictProbe_Status:"
            ),
            "Host mode changed {api}/{mode} native return"
        );
    }
    assert_eq!(
        stop.queries.load(Ordering::SeqCst),
        0,
        "unsupported calls must not enter Profile DNS transport"
    );
}

/// GetAddrInfoExW asynchronous event and callback modes are routed through
/// the Profile wire client.  The ANSI async ABI remains an explicit reject
/// because Microsoft documents those parameters as reserved for ExA.
#[test]
fn strict_getaddrinfoexw_async_routes_without_host_fallback() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let stop = spawn_fixture_dns(
        try_bind_fixture_dns().expect("fixture bind"),
        FixtureMode::DelayedAddress,
    );
    let root = std::env::temp_dir().join(format!("envbox-dns-exw-async-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let probe = probe_exe().expect("probe required");

    for mode in ["event", "callback", "close-event"] {
        let out = envbox_with_root(&root)
            .env("ENVBOX_RUNTIME_DLL", &dll)
            .args(["run", "--profile", &profile])
            .arg(&probe)
            .args(["--dns-strict", "ex-w", mode])
            .output()
            .expect("GetAddrInfoExW async probe");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{mode}: {out:?}");
        assert_eq!(field_after(&text, "StrictProbe_Status:"), "997", "{text}");
        assert_eq!(
            field_after(&text, "StrictProbe_InitialOverlappedStatus:"),
            "10036",
            "{mode} must publish WSAEINPROGRESS while the request is pending: {text}"
        );
        assert_eq!(
            field_after(&text, "StrictProbe_FinalCallbacks:"),
            if mode == "callback" { "1" } else { "0" },
            "{text}"
        );
        assert_eq!(field_after(&text, "StrictProbe_Records:"), "1", "{text}");
        if mode == "event" || mode == "close-event" {
            assert_eq!(
                field_after(&text, "StrictProbe_DrainStatus:"),
                "0",
                "{text}"
            );
            assert_eq!(
                field_after(&text, "StrictProbe_DrainStatusSecond:"),
                "0",
                "{text}"
            );
        } else {
            assert_eq!(
                field_after(&text, "StrictProbe_CallbackStatus:"),
                "0",
                "{text}"
            );
        }
    }
    assert!(stop.queries.load(Ordering::SeqCst) >= 1);
}

/// The callback may release every caller-owned async buffer before returning.
/// Runtime completion must have finished all borrowed writes before entering
/// user code and must not touch OVERLAPPED/result/name-handle afterwards.
#[test]
fn strict_getaddrinfoexw_callback_can_release_caller_storage() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let stop = spawn_fixture_dns(
        try_bind_fixture_dns().expect("fixture bind"),
        FixtureMode::Address,
    );
    let root =
        std::env::temp_dir().join(format!("envbox-dns-exw-callback-free-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = envbox_with_root(&root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile])
        .arg(probe_exe().expect("probe required"))
        .args(["--dns-strict", "ex-w", "callback-free"])
        .output()
        .expect("GetAddrInfoExW callback release probe");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(field_after(&text, "StrictProbe_Status:"), "997", "{text}");
    assert_eq!(field_after(&text, "StrictProbe_DrainEvent:"), "0", "{text}");
    assert_eq!(
        field_after(&text, "StrictProbe_CallbackStatus:"),
        "0",
        "{text}"
    );
    assert_eq!(
        field_after(&text, "StrictProbe_FinalCallbacks:"),
        "1",
        "{text}"
    );
    assert_eq!(field_after(&text, "StrictProbe_Records:"), "0", "{text}");
    assert!(stop.queries.load(Ordering::SeqCst) >= 1);
}

/// Cancellation must complete the ExW callback exactly once with the
/// documented WSA_E_CANCELLED status and no result chain.
#[test]
fn strict_getaddrinfoexw_async_cancel_is_single_completion() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let stop = spawn_fixture_dns(
        try_bind_fixture_dns().expect("fixture bind"),
        FixtureMode::Silent,
    );
    let root = std::env::temp_dir().join(format!("envbox-dns-exw-cancel-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = envbox_with_root(&root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile])
        .arg(probe_exe().expect("probe required"))
        .args(["--dns-strict", "ex-w", "cancel"])
        .output()
        .expect("GetAddrInfoExW cancel probe");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(field_after(&text, "StrictProbe_Status:"), "997", "{text}");
    assert_eq!(
        field_after(&text, "StrictProbe_InitialOverlappedStatus:"),
        "10036",
        "cancel must observe WSAEINPROGRESS before publishing its terminal status: {text}"
    );
    assert_eq!(field_after(&text, "StrictProbe_Cancel:"), "0", "{text}");
    assert_eq!(
        field_after(&text, "StrictProbe_CallbackStatus:"),
        "10111",
        "{text}"
    );
    assert_eq!(
        field_after(&text, "StrictProbe_FinalCallbacks:"),
        "1",
        "{text}"
    );
    assert_eq!(field_after(&text, "StrictProbe_Records:"), "0", "{text}");
    assert!(stop.queries.load(Ordering::SeqCst) <= 1);
}

/// Event completion must retire internal state even when callers inspect the
/// result through their own OVERLAPPED storage and do not call the helper.
#[test]
fn strict_getaddrinfoexw_event_releases_more_than_pending_limit() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let stop = spawn_fixture_dns(
        try_bind_fixture_dns().expect("fixture bind"),
        FixtureMode::Address,
    );
    let root = std::env::temp_dir().join(format!("envbox-dns-exw-repeat-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = envbox_with_root(&root)
        .env("ENVBOX_RUNTIME_DLL", &dll)
        .args(["run", "--profile", &profile])
        .arg(probe_exe().expect("probe required"))
        .args(["--dns-strict", "ex-w", "repeat"])
        .output()
        .expect("GetAddrInfoExW repeat probe");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        field_after(&text, "StrictProbe_RepeatCompleted:"),
        "80",
        "{text}"
    );
    assert!(stop.queries.load(Ordering::SeqCst) >= 1);
}

#[test]
fn typed_upstream_order_negative_answers_and_total_deadline() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    for (mode, name, expected, second_expected) in [
        (FixtureMode::Servfail, "rr.fixture.test", "0", 1),
        (FixtureMode::Refused, "rr.fixture.test", "0", 1),
        (FixtureMode::Nxdomain, "rr.fixture.test", "9003", 0),
        (FixtureMode::Nodata, NODATA_NAME, "9501", 0),
        (FixtureMode::Silent, "strict.fixture.test", "11002", 0),
    ] {
        let first = spawn_fixture_dns(try_bind_fixture_dns().expect("first fixture"), mode);
        let second_socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        second_socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let second_port = second_socket.local_addr().unwrap().port().to_string();
        let second = spawn_fixture_dns(second_socket, FixtureMode::Address);
        let root = std::env::temp_dir().join(format!("envbox-dns-order-{}", Uuid::new_v4()));
        let id = make_profile(&root, &["127.0.0.1"], true);
        let out = envbox_with_root(&root)
            .args([
                "profile",
                "dns",
                "add",
                &id,
                "--type",
                "udp",
                "--address",
                "127.0.0.1",
                "--port",
                &second_port,
            ])
            .output()
            .unwrap();
        assert!(out.status.success(), "second upstream: {out:?}");
        let start = Instant::now();
        let mut command = envbox_with_root(&root);
        command
            .env("ENVBOX_RUNTIME_DLL", &dll)
            .args(["run", "--profile", &id])
            .arg(probe_exe().unwrap());
        let key = if mode == FixtureMode::Silent {
            command.args(["--dns-strict", "ex-w", "deadline"]);
            "StrictProbe_Status:"
        } else {
            command.args(["--dns-rr", name, "1", "ex"]);
            "DnsRR_Status:"
        };
        let out = command.output().unwrap();
        assert_eq!(
            field_after(&String::from_utf8_lossy(&out.stdout), key),
            expected,
            "{mode:?}: {out:?}"
        );
        assert_eq!(
            first.queries.load(Ordering::SeqCst),
            1,
            "first upstream must be tried first"
        );
        assert_eq!(
            second.queries.load(Ordering::SeqCst),
            second_expected,
            "{mode:?}: fallback must respect authoritative negatives and total budget"
        );
        if mode == FixtureMode::Silent {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "deadline reset across upstreams: {:?}",
                start.elapsed()
            );
        }
    }
}

/// Every supported Windows DNS entry point must route every QTYPE to Profile.
#[test]
fn arbitrary_qtypes_route_to_profile_dns() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);
    let root = std::env::temp_dir().join(format!("envbox-dns-rr-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let mut failures = Vec::new();
    for api in ["a", "w", "utf8", "ex", "async"] {
        for kind in [65u16, 64, 16, 12, 5, 33, 65280, 1] {
            let out = envbox_with_root(&root)
                .env("ENVBOX_RUNTIME_DLL", &dll)
                .args(["run", "--profile", &profile_id])
                .arg(probe_exe().expect("probe required"))
                .args(["--dns-rr", "rr.fixture.test", &kind.to_string(), api])
                .output()
                .expect("run RR probe");
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !out.status.success()
                || !stdout.contains("DnsRR_Status:\n0")
                || !stdout.contains(&format!("DnsRR_Record: type={kind} "))
                || !stdout.contains("DnsRR_Freed:\ntrue")
            {
                failures.push(format!(
                    "api={api} type={kind}: {stdout} stderr={}",
                    String::from_utf8_lossy(&out.stderr)
                ));
                if std::env::var_os("ENVBOX_DNS_RR_RED").is_some() {
                    stop.store(true, Ordering::SeqCst);
                    panic!(
                        "fixture packets={}\n{}",
                        stop.queries.load(Ordering::SeqCst),
                        failures.join("\n")
                    );
                }
            }
            if kind == 16 && !stdout.contains("value=profile-marker") {
                failures.push(format!("TXT content lost ({api}): {stdout}"));
            }
            if matches!(kind, 5 | 12 | 33) && !stdout.contains("target.fixture.test") {
                failures.push(format!("name content lost ({api}, {kind}): {stdout}"));
            }
            if matches!(kind, 64 | 65) && !stdout.contains("value=000100") {
                failures.push(format!("SVCB/HTTPS content lost ({api}, {kind}): {stdout}"));
            }
            if kind == 65280 && !stdout.contains("value=deadbeef") {
                failures.push(format!("unknown RR content lost ({api}): {stdout}"));
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        failures.is_empty(),
        "fixture packets={}\n{}",
        stop.queries.load(Ordering::SeqCst),
        failures.join("\n")
    );
}

#[test]
fn arbitrary_qtypes_profile_error_does_not_fall_back() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::Address);
    let root = std::env::temp_dir().join(format!("envbox-dns-rr-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let mut failures = Vec::new();
    for api in ["a", "w", "utf8", "ex", "async"] {
        for (name, kind, expected) in [
            ("rr.servfail.test", 1u16, "9002"),
            ("rr.servfail.test", 65, "9002"),
            ("rr.servfail.test", 65280, "9002"),
            ("rr.mismatch.test", 65, "1460"),
            ("rr.wrongtype.test", 65, "1460"),
        ] {
            let out = envbox_with_root(&root)
                .env("ENVBOX_RUNTIME_DLL", &dll)
                .args(["run", "--profile", &profile_id])
                .arg(probe_exe().expect("probe required"))
                .args(["--dns-rr", name, &kind.to_string(), api])
                .output()
                .expect("run RR error probe");
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Native host querying this .test name yields a different error.
            // Preserving the Profile's SERVFAIL proves its result is returned.
            if !out.status.success()
                || field_after(&stdout, "DnsRR_Status:") != expected
                || field_after(&stdout, "DnsRR_Records:") != "0"
            {
                failures.push(format!("api={api} kind={kind}: {stdout}"));
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn arbitrary_qtypes_root_localhost_and_tcp() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let udp = spawn_fixture_dns(sock, FixtureMode::Truncated);
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, fixture_dns_port())).expect("TCP DNS fixture bind");
    listener.set_nonblocking(true).unwrap();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let tcp = TcpFixture {
        stop: stopping.clone(),
        thread: Some(std::thread::spawn(move || {
            let mut requests = 0;
            while !stop.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut length = [0u8; 2];
                    stream.read_exact(&mut length).unwrap();
                    let mut query = vec![0; u16::from_be_bytes(length) as usize];
                    stream.read_exact(&mut query).unwrap();
                    let reply = build_fixture_response(&query, FixtureMode::Address);
                    stream
                        .write_all(&(reply.len() as u16).to_be_bytes())
                        .unwrap();
                    stream.write_all(&reply).unwrap();
                    requests += 1;
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            requests
        })),
    };
    let root = std::env::temp_dir().join(format!("envbox-dns-rr-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let mut failures = Vec::new();
    for api in ["a", "w", "utf8", "ex", "async"] {
        for (name, kind, options) in [
            ("rr.fixture.test", 65, 0x108),
            ("rr.fixture.test", 65280, 0x10a),
            (".", 2, 0x108),
            ("localhost", 65, 0x108),
        ] {
            let out = envbox_with_root(&root)
                .env("ENVBOX_RUNTIME_DLL", &dll)
                .args(["run", "--profile", &profile_id])
                .arg(probe_exe().expect("probe required"))
                .args([
                    "--dns-rr",
                    name,
                    &kind.to_string(),
                    api,
                    &options.to_string(),
                ])
                .output()
                .expect("run TCP RR probe");
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !out.status.success()
                || !stdout.contains("DnsRR_Status:\n0")
                || !stdout.contains(&format!("DnsRR_Record: type={kind} "))
            {
                failures.push(format!(
                    "api={api} name={name} kind={kind} options={options}: {stdout}"
                ));
            }
        }
    }
    let address = run_probe_resolve(&root, &dll, &profile_id, "fixture.test");
    assert!(
        String::from_utf8_lossy(&address.stdout).contains("10.99.0.1"),
        "AF_UNSPEC must follow UDP TC to TCP: {address:?}"
    );
    let udp_before = udp.queries.load(Ordering::SeqCst);
    let port = fixture_dns_port().to_string();
    for args in [
        vec![
            "profile",
            "dns",
            "add",
            &profile_id,
            "--type",
            "tcp",
            "--address",
            "127.0.0.1",
            "--port",
            &port,
        ],
        vec!["profile", "dns", "remove", &profile_id, "--index", "0"],
    ] {
        let out = envbox_with_root(&root)
            .args(args)
            .output()
            .expect("typed TCP configuration");
        assert!(out.status.success(), "typed TCP config: {out:?}");
    }
    for api in ["a", "w", "utf8", "ex", "async"] {
        let out = envbox_with_root(&root)
            .env("ENVBOX_RUNTIME_DLL", &dll)
            .args(["run", "--profile", &profile_id])
            .arg(probe_exe().unwrap())
            .args(["--dns-rr", "rr.fixture.test", "65", api])
            .output()
            .unwrap();
        assert_eq!(
            field_after(&String::from_utf8_lossy(&out.stdout), "DnsRR_Status:"),
            "0",
            "{out:?}"
        );
    }
    assert_eq!(
        udp.queries.load(Ordering::SeqCst),
        udp_before,
        "typed TCP must not send UDP"
    );
    stopping.store(true, Ordering::SeqCst);
    assert_eq!(
        tcp.finish(),
        27,
        "20 RR + AF_UNSPEC A/AAAA + 5 typed TCP queries"
    );
    udp.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn dnsquery_numeric_literals_and_cname_chain() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::CnameChain);
    let root = std::env::temp_dir().join(format!("envbox-dns-rr-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let mut failures = Vec::new();
    for api in ["a", "w", "utf8", "ex", "async"] {
        for (name, kind, expected, options) in [
            ("127.0.0.1", 1, "value=127.0.0.1", 0),
            ("::1", 28, "type=28 ", 0),
            ("127.0.0.1", 1, "value=127.0.0.1", 0x108),
            ("::1", 28, "type=28 ", 0x108),
            (CNAME_A_NAME, 1, "value=10.99.0.1", 0),
        ] {
            let out = envbox_with_root(&root)
                .env("ENVBOX_RUNTIME_DLL", &dll)
                .args(["run", "--profile", &profile_id])
                .arg(probe_exe().expect("probe required"))
                .args(["--dns-rr", name, &kind.to_string(), api])
                .arg(options.to_string())
                .output()
                .expect("run literal/CNAME probe");
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !out.status.success()
                || !stdout.contains("DnsRR_Status:\n0")
                || !stdout.contains(expected)
            {
                failures.push(format!("api={api} name={name}: {stdout}"));
            }
            if api == "async" && name != CNAME_A_NAME && !stdout.contains("DnsRR_ReturnStatus:\n0")
            {
                failures.push(format!("numeric literal must complete inline: {stdout}"));
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn dnsquery_tcp_timeout_and_cancel_stay_in_profile() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let listener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, fixture_dns_port())).expect("TCP DNS fixture bind");
    listener.set_nonblocking(true).unwrap();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let tcp = TcpFixture {
        stop: stopping.clone(),
        thread: Some(std::thread::spawn(move || {
            let mut requests = 0;
            while !stop.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut prefix = [0; 2];
                    if stream.read_exact(&mut prefix).is_ok() {
                        let mut query = vec![0; u16::from_be_bytes(prefix) as usize];
                        if stream.read_exact(&mut query).is_ok() {
                            requests += 1;
                            // Hold the reply until the client times out or cancels.
                            let _ = stream.read(&mut prefix);
                        }
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            requests
        })),
    };
    let root = std::env::temp_dir().join(format!("envbox-dns-rr-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let mut failures = Vec::new();
    for (api, cancel, expected) in [
        ("ex", false, "1460"),
        ("async", false, "1460"),
        ("async", true, "1223"),
    ] {
        let mut command = envbox_with_root(&root);
        command
            .env("ENVBOX_RUNTIME_DLL", &dll)
            .args(["run", "--profile", &profile_id])
            .arg(probe_exe().expect("probe required"))
            .args(["--dns-rr", "rr.fixture.test", "65", api, "266"]);
        if cancel {
            command.arg("--cancel");
        }
        let out = command.output().expect("run TCP failure probe");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !out.status.success()
            || field_after(&stdout, "DnsRR_Status:") != expected
            || field_after(&stdout, "DnsRR_Records:") != "0"
        {
            failures.push(format!("api={api} cancel={cancel}: {stdout}"));
        }
        if cancel && field_after(&stdout, "DnsRR_CancelStatus:") != "0" {
            failures.push(format!("cancel failed: {stdout}"));
        }
    }
    stopping.store(true, Ordering::SeqCst);
    assert_eq!(tcp.finish(), 3, "every request must reach Profile TCP");
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
    assert_ne!(
        got, "10.99.0.1",
        "must not invent fixture answer:\n{stdout}"
    );
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

/// A truncated Profile response returns a retryable error.
#[test]
fn truncated_response_returns_profile_error() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::Truncated);
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = run_probe_resolve(&root, &dll, &profile, "rr.truncated.test");
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "{out:?}");
    let value = field_after(&String::from_utf8_lossy(&out.stdout), "getaddrinfo:");
    assert_eq!(
        value, "<error 11002>",
        "incomplete Profile resolution must return WSATRY_AGAIN: {value}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// An incomplete CNAME chain returns a retryable error, not NXDOMAIN.
#[test]
fn cname_only_returns_retryable_profile_error() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::CnameOnly);
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = run_probe_resolve(&root, &dll, &profile, "cname-only.test");
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "{out:?}");
    let value = field_after(&String::from_utf8_lossy(&out.stdout), "getaddrinfo:");
    assert_eq!(
        value, "<error 11002>",
        "incomplete Profile resolution must return WSATRY_AGAIN: {value}"
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

/// An unreachable Profile server returns a retryable error.
#[test]
fn unreachable_returns_retryable_profile_error() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["192.0.2.1"], true);
    let out = run_probe_resolve(&root, &dll, &profile, "unreachable.fixture.test");
    assert!(out.status.success(), "{out:?}");
    let value = field_after(&String::from_utf8_lossy(&out.stdout), "getaddrinfo:");
    assert_eq!(
        value, "<error 11002>",
        "unreachable Profile resolution must return WSATRY_AGAIN: {value}"
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

/// Asynchronous DnsQueryEx must route through the Profile wire client and
/// preserve the caller callback contract. The unique name avoids a prior
/// synchronous test's positive cache entry hiding the fixture request.
#[test]
fn dnsquery_ex_async_smoke_routes_fixture() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    // Hold each answer long enough to observe the real pending result field
    // before either callback can publish its completion status.
    let stop = spawn_fixture_dns(sock, FixtureMode::DelayedAddress);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_dnsquery_ex_async(
        &root,
        &dll,
        &profile_id,
        ASYNC_FIXTURE_NAME,
        false,
        false,
        true,
    );
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let got = field_after(&stdout, "DnsQueryEx_A_Async:");
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReturnStatus:"),
        "9506",
        "custom async DnsQueryEx must return DNS_REQUEST_PENDING:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_WaitStatus:"),
        "0",
        "custom async DnsQueryEx must complete within the bounded wait:\n{stdout}"
    );
    assert_eq!(
        got, "10.99.0.1",
        "async DnsQueryEx must route the fixture via Profile DNS:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_Callbacks:"),
        "1",
        "async DnsQueryEx must invoke the caller callback exactly once:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_CallbackCancelStatus:"),
        "87",
        "callback-side cancel of the completed generation must stay local:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReentryReturnStatus:"),
        "9506",
        "callback re-entry must create a second pending request:\n{stdout}"
    );
    let reentry_initial = field_after(&stdout, "DnsQueryEx_A_Async_ReentryInitialQueryStatus:");
    assert!(
        reentry_initial == "9506" || reentry_initial == "0",
        "re-entry must expose the result field observed after DnsQueryEx (pending or an already-completed callback):\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReentryWaitStatus:"),
        "0",
        "re-entry callback must complete within the bounded wait:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReentryCallbacks:"),
        "1",
        "re-entry must invoke its callback exactly once:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReentryCallbackStatus:"),
        "0",
        "re-entry callback must report success:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_StaleCancelStatus:"),
        "87",
        "a copied old generation must not cancel the re-entered request:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_Reentry:"),
        "10.99.0.1",
        "re-entry must use the custom Profile route:\n{stdout}"
    );
    std::thread::sleep(Duration::from_millis(600));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dnsquery_ex_local_name_keeps_native_synchronous_completion() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);
    let out =
        run_probe_dnsquery_ex_async(&root, &dll, &profile_id, "localhost", false, false, false);
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReturnStatus:"),
        "0",
        "{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_Callbacks:"),
        "0",
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Cancellation must still produce exactly one completion callback. The
/// delayed fixture ensures the cancellation request races a pending query,
/// exercising the worker's single callback-owned release point.
#[test]
fn dnsquery_ex_async_cancel_completes_callback() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let _guard = lock_fixture();
    let Some(sock) = try_bind_fixture_dns() else {
        return;
    };
    let stop = spawn_fixture_dns(sock, FixtureMode::DelayedAddress);

    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile_id = make_profile(&root, &["127.0.0.1"], true);

    let out = run_probe_dnsquery_ex_async(
        &root,
        &dll,
        &profile_id,
        ASYNC_CANCEL_NAME,
        true,
        true,
        false,
    );
    stop.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(out.status.success(), "run failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_ReturnStatus:"),
        "9506",
        "custom async DnsQueryEx must return DNS_REQUEST_PENDING:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_WaitStatus:"),
        "0",
        "cancelled custom async DnsQueryEx must complete within the bounded wait:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_Callbacks:"),
        "1",
        "cancelled async DnsQueryEx must invoke one completion callback:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_CancelStatus:"),
        "0",
        "DnsCancelQuery must accept the Runtime-owned cancel handle:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_CancelCopied:"),
        "1",
        "DnsCancelQuery must accept a copied Runtime-owned cancel handle:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_CallbackCancelStatus:"),
        "87",
        "callback-side cancel after concurrent cancellation must remain local:\n{stdout}"
    );
    assert_eq!(
        field_after(&stdout, "DnsQueryEx_A_Async_InitialQueryStatus:"),
        "9506",
        "pending query must publish DNS_REQUEST_PENDING before return:\n{stdout}"
    );
    assert_ne!(
        field_after(&stdout, "DnsQueryEx_A_Async_CallbackStatus:"),
        "0",
        "cancelled query must report a non-success completion status:\n{stdout}"
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

/// DnsQueryEx preserves a Profile referral error without consulting Host DNS.
#[test]
fn dnsquery_ex_referral_does_not_fall_back() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::Referral);
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let virtual_id = make_profile(&root, &["127.0.0.1"], true);
    let out = run_probe_dnsquery_ex(&root, &dll, &virtual_id, REFERRAL_NAME);
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "{out:?}");
    let value = field_after(&String::from_utf8_lossy(&out.stdout), "DnsQueryEx_A:");
    assert!(
        value.starts_with("<error "),
        "referral must return a Profile error: {value}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// An NS-only referral returns a retryable Profile error.
#[test]
fn getaddrinfo_referral_returns_retryable_profile_error() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::Referral);
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = run_probe_resolve(&root, &dll, &profile, "referral.test");
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "{out:?}");
    let value = field_after(&String::from_utf8_lossy(&out.stdout), "getaddrinfo:");
    assert_eq!(
        value, "<error 11002>",
        "incomplete Profile resolution must return WSATRY_AGAIN: {value}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Chromium reads search/domain/devolution policy from HKLM in addition to
/// GetAdaptersAddresses.  The Profile currently models DNS servers only, so
/// VirtualView must hide the host Domain value rather than combine it with the
/// Profile server list.  The host value is an empty REG_SZ on the acceptance
/// image; skip this red-capable check on images where the value is absent.
#[test]
fn virtual_view_hides_chromium_host_dns_policy() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _probe = probe_exe().expect("envbox-probe.exe required");
    let root = std::env::temp_dir().join(format!("envbox-dns-policy-test-{}", Uuid::new_v4()));
    let virtual_id = make_profile(&root, &["127.0.0.1"], true);
    let virtual_out = run_probe_dns_system_settings(&root, &dll, &virtual_id);
    assert!(
        virtual_out.status.success(),
        "virtual run failed: {virtual_out:?}"
    );
    let virtual_stdout = String::from_utf8_lossy(&virtual_out.stdout);
    let virtual_domain = field_after(&virtual_stdout, "Domain:");
    assert_eq!(
        virtual_domain, "<missing>",
        "VirtualView must hide host Chromium DNS Domain policy:\n{virtual_stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// DnsQuery_A under TC must report the Profile transport error.
#[test]
fn dnsquery_a_truncated_does_not_fall_back() {
    let dll = test_runtime_dll().expect("runtime DLL required");
    let _guard = lock_fixture();
    let sock = try_bind_fixture_dns().expect("DNS acceptance fixture must bind");
    let stop = spawn_fixture_dns(sock, FixtureMode::Truncated);
    let root = std::env::temp_dir().join(format!("envbox-dns-test-{}", Uuid::new_v4()));
    let profile = make_profile(&root, &["127.0.0.1"], true);
    let out = run_probe_dnsquery(&root, &dll, &profile, "rr.truncated.test");
    stop.store(true, Ordering::SeqCst);
    assert!(out.status.success(), "{out:?}");
    let value = field_after(&String::from_utf8_lossy(&out.stdout), "DnsQuery_A:");
    assert!(
        value.starts_with("<error "),
        "TC must return a Profile error: {value}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
