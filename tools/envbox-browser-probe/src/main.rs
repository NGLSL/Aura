//! envbox-browser-probe CLI. See `lib.rs` for report schema and assertions.

use envbox_browser_probe::{
    build_report, chromium_ip_handling_switch, evaluate_assertions, evaluate_browser_ice,
    has_webrtc_ip_handling_switch, intl_format_sample_mismatches, intl_locale_mismatches,
    parse_browser_ice_report, parse_stun_binding_reply, policy_effective_from, policy_env_from, BrowserIceReport,
    LocalAddress, PolicyEffective, PolicyEnv, Report, StunCandidate, STUN_MAGIC,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

fn print_help() {
    eprintln!("envbox-browser-probe — network / WebRTC path report");
    eprintln!();
    eprintln!(
        "usage: envbox-browser-probe [--json] [--text] [--stun HOST:PORT] \
         [--stun-optional] [--expect-policy TOKEN] [--assert] \
         [--write-page PATH] [--browser [EXE]] [--browser-stun HOST:PORT] \
         [--expect-intl-locale L] [--expect-intl-tz TZ]"
    );
    eprintln!();
    eprintln!("  --json              JSON report to stdout (default)");
    eprintln!("  --text              human-readable report to stdout");
    eprintln!("  --stun ADDR         raw UDP STUN Binding (Network Guard oracle)");
    eprintln!("  --stun-optional     do not fail exit code when STUN errors");
    eprintln!("  --expect-policy T   acceptance policy (default: observed)");
    eprintln!("  --assert            exit 2 when policy expectations are violated");
    eprintln!("  --write-page PATH   write webrtc-intl-probe.html acceptance page");
    eprintln!("  --browser [EXE]     run Chromium/Edge headless on the page (RTCPeerConnection + Intl)");
    eprintln!("  --browser-stun A    STUN server for the in-page ICE gather");
    eprintln!("  --browser-arg X     extra Chromium/Edge arg (repeatable)");
    eprintln!("  --expect-intl-locale L  assert every Intl/navigator locale field");
    eprintln!("  --expect-intl-tz TZ     assert Intl timeZone");
    eprintln!();
    eprintln!("  --browser auto-injects --force-webrtc-ip-handling-policy from");
    eprintln!("  --expect-policy / ENVBOX_WEBRTC_POLICY unless already passed.");
}

const BROWSER_PAGE: &str = include_str!("../assets/webrtc-intl-probe.html");

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
    req[0] = 0x00;
    req[1] = 0x01; // Binding Request
    req[4..8].copy_from_slice(&STUN_MAGIC);
    let mut txid = [0u8; 12];
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    for i in 0..12 {
        txid[i] = ((nanos >> (i * 2)) & 0xff) as u8 ^ (0xA5 + i as u8);
    }
    req[8..20].copy_from_slice(&txid);
    sock.send_to(&req, addr).map_err(|e| format!("send: {e}"))?;

    let mut buf = [0u8; 256];
    let (n, _) = sock.recv_from(&mut buf).map_err(|e| format!("recv: {e}"))?;
    let (ip, port) = parse_stun_binding_reply(&buf[..n], &txid)?;
    let reflexive = SocketAddr::new(ip, port).to_string();
    Ok(StunCandidate {
        server: server.to_string(),
        transport: "udp".into(),
        local,
        reflexive,
        reflexive_class: envbox_browser_probe::classify_ip(ip),
    })
}

fn find_browser_exe(explicit: Option<String>) -> Result<std::path::PathBuf, String> {
    if let Some(p) = explicit {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!("browser exe not found: {}", pb.display()));
    }
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    for c in candidates {
        let p = std::path::PathBuf::from(c);
        if p.exists() {
            return Ok(p);
        }
    }
    Err("no Edge/Chrome found; pass --browser EXE".into())
}

/// Run the acceptance page in headless Chromium/Edge and scrape the JSON blob.
fn run_browser_probe(
    browser: Option<String>,
    stun: Option<String>,
    extra_args: &[String],
) -> Result<BrowserIceReport, String> {
    let exe = find_browser_exe(browser)?;
    let tmp = std::env::temp_dir().join("envbox-webrtc-intl-probe.html");
    std::fs::write(&tmp, BROWSER_PAGE).map_err(|e| format!("write page: {e}"))?;

    let mut url = format!("file:///{}", tmp.display().to_string().replace('\\', "/"));
    if let Some(s) = &stun {
        // Page already reads ?stun= — no source rewrite.
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str("stun=");
        url.push_str(&urlencoding_min(s));
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--dump-dom")
        .arg("--virtual-time-budget=12000");
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(&url);
    let output = cmd.output().map_err(|e| format!("launch browser: {e}"))?;
    let dom = String::from_utf8_lossy(&output.stdout);
    scrape_browser_json(&dom)
}

fn urlencoding_min(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | ':' | '[' | ']' => {
                c.to_string()
            }
            other => format!("%{:02X}", other as u8),
        })
        .collect()
}

fn scrape_browser_json(dom: &str) -> Result<BrowserIceReport, String> {
    // <pre id="envbox-probe-result">{...}</pre>
    let marker = "id=\"envbox-probe-result\"";
    let Some(start) = dom.find(marker) else {
        return Err("probe result element missing from DOM".into());
    };
    let after = &dom[start + marker.len()..];
    let Some(gt) = after.find('>') else {
        return Err("probe result element truncated".into());
    };
    let body = &after[gt + 1..];
    let Some(end) = body.find("</pre>") else {
        return Err("probe result not closed".into());
    };
    let json = body[..end].trim();
    // HTML-escape safety: the page writes raw JSON text.
    let json = json.replace("&quot;", "\"").replace("&amp;", "&");
    parse_browser_ice_report(&json)
}

fn main() {
    let mut json_mode = true;
    let mut stun: Option<String> = None;
    let mut stun_optional = false;
    let mut assert_mode = false;
    let mut expect_policy: Option<PolicyEffective> = None;
    let mut write_page: Option<String> = None;
    let mut browser: Option<Option<String>> = None;
    let mut browser_stun: Option<String> = None;
    let mut expect_intl_locale: Option<String> = None;
    let mut expect_intl_tz: Option<String> = None;
    let mut browser_extra: Vec<String> = Vec::new();

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
            "--write-page" => {
                i += 1;
                write_page = args.get(i).cloned();
            }
            "--browser" => {
                // Optional EXE path in the next slot when it is not a flag.
                if let Some(next) = args.get(i + 1) {
                    if !next.starts_with('-') {
                        browser = Some(Some(next.clone()));
                        i += 1;
                    } else {
                        browser = Some(None);
                    }
                } else {
                    browser = Some(None);
                }
            }
            "--browser-stun" => {
                i += 1;
                browser_stun = args.get(i).cloned();
            }
            "--expect-intl-locale" => {
                i += 1;
                expect_intl_locale = args.get(i).cloned();
            }
            "--expect-intl-tz" => {
                i += 1;
                expect_intl_tz = args.get(i).cloned();
            }
            "--browser-arg" => {
                i += 1;
                if let Some(a) = args.get(i) {
                    browser_extra.push(a.clone());
                }
            }
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

    if let Some(path) = write_page {
        if let Err(e) = std::fs::write(&path, BROWSER_PAGE) {
            eprintln!("write-page: {e}");
            std::process::exit(1);
        }
        eprintln!("wrote {}", path);
    }

    let env: PolicyEnv = policy_env_from(|k| std::env::var(k).ok());

    // Auto-inject Chromium WebRTC switch so --browser + --expect-policy is a
    // one-command acceptance. Never overwrite an explicit --browser-arg.
    if browser.is_some() {
        let effective = expect_policy.unwrap_or(policy_effective_from(&env));
        if let Some(val) = chromium_ip_handling_switch(effective) {
            if !has_webrtc_ip_handling_switch(&browser_extra) {
                browser_extra.push(format!("--force-webrtc-ip-handling-policy={val}"));
            }
        }
    }

    let mut browser_report: Option<BrowserIceReport> = None;
    if let Some(exe) = browser {
        match run_browser_probe(exe, browser_stun, &browser_extra) {
            Ok(r) => browser_report = Some(r),
            Err(e) => {
                eprintln!("browser probe failed: {e}");
                std::process::exit(1);
            }
        }
    }
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

    // Merge browser ICE/Intl into the printed JSON without breaking the schema.
    if json_mode {
        match serde_json::to_value(&report) {
            Ok(mut v) => {
                if let Some(b) = &browser_report {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert(
                            "browser_ice".into(),
                            serde_json::to_value(b).unwrap_or_default(),
                        );
                    }
                }
                match serde_json::to_string_pretty(&v) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("json error: {e}");
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("json error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        print!("{}", report.render_text());
        if let Some(b) = &browser_report {
            println!("=== BROWSER ICE / INTL ===");
            println!("{}", serde_json::to_string_pretty(b).unwrap_or_default());
        }
    }

    if let Some(err) = &report.stun_error {
        if report.stun_candidates.is_empty() && !stun_optional {
            eprintln!("stun failed: {err}");
            std::process::exit(1);
        }
    }

    if assert_mode {
        let expected = expect_policy.unwrap_or(report.policy_effective);
        let mut violations = evaluate_assertions(&report, expected);
        if let Some(b) = &browser_report {
            violations.extend(evaluate_browser_ice(b, expected));
            if let Some(loc) = &expect_intl_locale {
                violations.extend(intl_locale_mismatches(b, loc));
                violations.extend(intl_format_sample_mismatches(b, loc));
            }
            if let Some(tz) = &expect_intl_tz {
                if b.intl_timezone != *tz {
                    violations.push(format!(
                        "intl-tz: expected {tz:?}, got {:?}",
                        b.intl_timezone
                    ));
                }
            }
        }
        for v in &violations {
            eprintln!("assert: {v}");
        }
        if !violations.is_empty() {
            std::process::exit(2);
        }
    }
}
