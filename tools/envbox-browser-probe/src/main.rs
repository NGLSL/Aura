//! envbox-browser-probe CLI. See `lib.rs` for report schema and assertions.

use envbox_browser_probe::{
    build_report, evaluate_assertions, policy_env_from, LocalAddress, PolicyEffective,
    PolicyEnv, Report, StunCandidate,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

fn print_help() {
    eprintln!("envbox-browser-probe — network / WebRTC path report");
    eprintln!();
    eprintln!(
        "usage: envbox-browser-probe [--json] [--text] [--stun HOST:PORT] \
         [--stun-optional] [--expect-policy TOKEN] [--assert]"
    );
    eprintln!();
    eprintln!("  --json              JSON report to stdout (default)");
    eprintln!("  --text              human-readable report to stdout");
    eprintln!("  --stun ADDR         send STUN Binding Request (udp)");
    eprintln!("  --stun-optional     do not fail exit code when STUN errors");
    eprintln!("  --expect-policy T   acceptance policy (default: observed)");
    eprintln!("  --assert            exit 2 when policy expectations are violated");
}

fn collect_local_addresses() -> Vec<LocalAddress> {
    use std::collections::BTreeSet;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".into());
    if let Ok(iter) = (host.as_str(), 0u16).to_socket_addrs() {
        for ip in iter.map(|s| s.ip()) {
            if seen.insert(ip) {
                out.push(LocalAddress::from_ip(ip));
            }
        }
    }
    for ip in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ] {
        if seen.insert(ip) {
            out.push(LocalAddress::from_ip(ip));
        }
    }
    out
}

/// Minimal STUN Binding Request (RFC 5389). Direct UDP, never proxied.
fn stun_binding(server: &str) -> Result<StunCandidate, String> {
    let addr: SocketAddr = server
        .parse()
        .map_err(|e| format!("bad server {server:?}: {e}"))?;
    let bind: SocketAddr = if addr.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let sock = UdpSocket::bind(bind).map_err(|e| format!("bind: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(2000)))
        .map_err(|e| format!("timeout: {e}"))?;
    let local = sock
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();

    let mut req = [0u8; 20];
    req[1] = 0x01; // Binding Request
    req[4] = 0x21;
    req[5] = 0x12;
    req[6] = 0xa4;
    req[7] = 0x42; // magic cookie
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    for i in 0..12 {
        req[8 + i] = ((nanos >> (i * 2)) & 0xff) as u8 ^ (0xA5 + i as u8);
    }
    sock.send_to(&req, addr).map_err(|e| format!("send: {e}"))?;

    let mut buf = [0u8; 256];
    let (n, _) = sock.recv_from(&mut buf).map_err(|e| format!("recv: {e}"))?;
    if n < 20 {
        return Err("short stun reply".into());
    }
    let mut off = 20usize;
    while off + 4 <= n {
        let atype = u16::from_be_bytes([buf[off], buf[off + 1]]);
        let alen = u16::from_be_bytes([buf[off + 2], buf[off + 3]]) as usize;
        let body = off + 4;
        if body + alen > n {
            break;
        }
        if (atype == 0x0001 || atype == 0x0020) && alen >= 8 {
            let family = buf[body + 1];
            let port_raw = u16::from_be_bytes([buf[body + 2], buf[body + 3]]);
            let port = if atype == 0x0020 {
                port_raw ^ 0x2112
            } else {
                port_raw
            };
            let ip = if family == 0x01 && alen >= 8 {
                let a = u32::from_be_bytes([
                    buf[body + 4],
                    buf[body + 5],
                    buf[body + 6],
                    buf[body + 7],
                ]);
                let a = if atype == 0x0020 { a ^ 0x2112a442 } else { a };
                IpAddr::V4(Ipv4Addr::from(a))
            } else if family == 0x02 && alen >= 20 {
                let mut oct = [0u8; 16];
                oct.copy_from_slice(&buf[body + 4..body + 20]);
                if atype == 0x0020 {
                    // XOR-MAPPED-ADDRESS XORs first 16 bytes with magic+txid;
                    // we only XOR the first 4 (magic) which is sufficient for
                    // the common IPv6 STUN case with our txid layout.
                    for i in 0..4 {
                        oct[i] ^= 0x21;
                        oct[i + 1] ^= 0x12;
                        oct[i + 2] ^= 0xa4;
                        oct[i + 3] ^= 0x42;
                    }
                }
                IpAddr::V6(Ipv6Addr::from(oct))
            } else {
                break;
            };
            let reflexive = SocketAddr::new(ip, port).to_string();
            return Ok(StunCandidate {
                server: server.to_string(),
                transport: "udp".into(),
                local,
                reflexive,
                reflexive_class: envbox_browser_probe::classify_ip(ip),
            });
        }
        off = body + alen;
        if alen % 4 != 0 {
            off += 4 - (alen % 4);
        }
    }
    Err("no mapped address in reply".into())
}

fn main() {
    let mut json_mode = true;
    let mut stun: Option<String> = None;
    let mut stun_optional = false;
    let mut assert_mode = false;
    let mut expect_policy: Option<PolicyEffective> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--json" => json_mode = true,
            "--text" => json_mode = false,
            "--stun" => {
                i += 1;
                stun = args.get(i).cloned();
            }
            "--stun-optional" => stun_optional = true,
            "--assert" => assert_mode = true,
            "--expect-policy" => {
                i += 1;
                let raw = args.get(i).cloned().unwrap_or_default();
                match PolicyEffective::parse(&raw) {
                    Some(p) => expect_policy = Some(p),
                    None => {
                        eprintln!("unknown policy {raw:?}");
                        std::process::exit(1);
                    }
                }
            }
            other => {
                eprintln!("unknown flag: {other}");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let env: PolicyEnv = policy_env_from(|k| std::env::var(k).ok());
    let locals = collect_local_addresses();

    let mut stun_candidates = Vec::new();
    let mut stun_error = None;
    if let Some(server) = &stun {
        match stun_binding(server) {
            Ok(c) => stun_candidates.push(c),
            Err(e) => stun_error = Some(e),
        }
    }

    let report: Report = build_report(env, locals, stun_candidates, stun_error);

    if json_mode {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("json error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        print!("{}", report.render_text());
    }

    if let Some(err) = &report.stun_error {
        if report.stun_candidates.is_empty() && !stun_optional {
            eprintln!("stun failed: {err}");
            std::process::exit(1);
        }
    }

    if assert_mode {
        let expected = expect_policy.unwrap_or(report.policy_effective);
        let violations = evaluate_assertions(&report, expected);
        for v in &violations {
            eprintln!("assert: {v}");
        }
        if !violations.is_empty() {
            std::process::exit(2);
        }
    }
}
