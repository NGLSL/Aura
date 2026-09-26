//! envbox-browser-probe: network / WebRTC path acceptance probe.
//!
//! Separate oracle from `envbox-probe` (environment view): this tool only
//! reports the network path a process tree can observe — local addresses,
//! optional STUN server-reflexive candidates, and the Browser Policy
//! artifacts EnvBox placed in the environment. It never modifies host
//! firewall, proxy, or system settings, and never fabricates results.

use envbox_core::WebRtcPolicy;
use serde::Serialize;

/// Address classification used throughout the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IpClass {
    Loopback,
    LinkLocal,
    Private,
    Public,
}

impl IpClass {
    pub fn as_str(self) -> &'static str {
        match self {
            IpClass::Loopback => "loopback",
            IpClass::LinkLocal => "link_local",
            IpClass::Private => "private",
            IpClass::Public => "public",
        }
    }
}

/// IP family label for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IpFamily {
    Ipv4,
    Ipv6,
}

/// Classify an IPv4/IPv6 unicast address.
///
/// - `loopback`: 127.0.0.0/8, ::1
/// - `link_local`: 169.254.0.0/16, fe80::/10
/// - `private`: RFC1918, CGNAT 100.64/10 (shared, not Internet-routable),
///   IPv6 ULA fc00::/7; IPv4-mapped IPv6 delegates to the embedded IPv4
/// - `public`: everything else
pub fn classify_ip(ip: std::net::IpAddr) -> IpClass {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => classify_v4(v4.octets()),
        IpAddr::V6(v6) => classify_v6(v6.octets()),
    }
}

fn classify_v4(o: [u8; 4]) -> IpClass {
    if o[0] == 127 {
        IpClass::Loopback
    } else if o[0] == 169 && o[1] == 254 {
        IpClass::LinkLocal
    } else if o[0] == 10
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 100 && (64..=127).contains(&o[1]))
    {
        IpClass::Private
    } else {
        IpClass::Public
    }
}

fn classify_v6(o: [u8; 16]) -> IpClass {
    // ::1
    if o[..15].iter().all(|&b| b == 0) && o[15] == 1 {
        return IpClass::Loopback;
    }
    // IPv4-mapped ::ffff:a.b.c.d — classify by the embedded IPv4 address.
    if o[..10].iter().all(|&b| b == 0) && o[10] == 0xff && o[11] == 0xff {
        return classify_v4([o[12], o[13], o[14], o[15]]);
    }
    // fe80::/10
    if o[0] == 0xfe && (o[1] & 0xc0) == 0x80 {
        return IpClass::LinkLocal;
    }
    // fc00::/7 unique-local
    if (o[0] & 0xfe) == 0xfc {
        return IpClass::Private;
    }
    IpClass::Public
}

/// STUN magic cookie (RFC 5389 §6).
pub const STUN_MAGIC: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];

/// Decode MAPPED-ADDRESS (0x0001) / XOR-MAPPED-ADDRESS (0x0020) attribute body.
///
/// RFC 5389 §15.2: XOR-MAPPED-ADDRESS XORs the port with the top 16 bits of the
/// magic cookie. IPv4 address is XORed with the magic cookie only; IPv6 address
/// is XORed with `magic cookie || transaction id` (full 16-byte mask).
pub fn decode_mapped_address(
    attr_type: u16,
    body: &[u8],
    txid: &[u8; 12],
) -> Option<(std::net::IpAddr, u16)> {
    if body.len() < 8 {
        return None;
    }
    let family = body[1];
    let port_raw = u16::from_be_bytes([body[2], body[3]]);
    let port = if attr_type == 0x0020 {
        port_raw ^ 0x2112
    } else {
        port_raw
    };
    if family == 0x01 && body.len() >= 8 {
        let mut a = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
        if attr_type == 0x0020 {
            a ^= 0x2112a442;
        }
        return Some((std::net::IpAddr::V4(std::net::Ipv4Addr::from(a)), port));
    }
    if family == 0x02 && body.len() >= 20 {
        let mut oct = [0u8; 16];
        oct.copy_from_slice(&body[4..20]);
        if attr_type == 0x0020 {
            // XOR with magic cookie || transaction id (RFC 5389 §15.2).
            for i in 0..4 {
                oct[i] ^= STUN_MAGIC[i];
            }
            for i in 0..12 {
                oct[4 + i] ^= txid[i];
            }
        }
        return Some((
            std::net::IpAddr::V6(std::net::Ipv6Addr::from(oct)),
            port,
        ));
    }
    None
}

/// Parse a STUN Binding Success response for XOR-MAPPED-ADDRESS / MAPPED-ADDRESS.
/// `reply` is the full UDP payload; `txid` must match the request transaction id.
pub fn parse_stun_binding_reply(
    reply: &[u8],
    txid: &[u8; 12],
) -> Result<(std::net::IpAddr, u16), String> {
    if reply.len() < 20 {
        return Err("short stun reply".into());
    }
    let msg_type = u16::from_be_bytes([reply[0], reply[1]]);
    // Binding Success Response = 0x0101
    if msg_type != 0x0101 {
        return Err(format!("unexpected stun type {msg_type:#06x}"));
    }
    if reply[4..8] != STUN_MAGIC {
        return Err("bad magic cookie".into());
    }
    if reply[8..20] != *txid {
        return Err("transaction id mismatch".into());
    }
    let mut off = 20usize;
    while off + 4 <= reply.len() {
        let atype = u16::from_be_bytes([reply[off], reply[off + 1]]);
        let alen = u16::from_be_bytes([reply[off + 2], reply[off + 3]]) as usize;
        let body = off + 4;
        if body + alen > reply.len() {
            break;
        }
        if atype == 0x0001 || atype == 0x0020 {
            if let Some(v) = decode_mapped_address(atype, &reply[body..body + alen], txid) {
                return Ok(v);
            }
        }
        off = body + alen;
        if alen % 4 != 0 {
            off += 4 - (alen % 4);
        }
    }
    Err("no mapped address in reply".into())
}

/// Real-browser ICE / Intl probe result (from `webrtc-intl-probe.html`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
pub struct BrowserIceReport {
    /// `navigator.language`
    pub navigator_language: String,
    /// `navigator.languages`
    pub navigator_languages: Vec<String>,
    /// `Intl.DateTimeFormat().resolvedOptions().locale`
    pub intl_date_time_locale: String,
    /// `Intl.NumberFormat().resolvedOptions().locale`
    pub intl_number_locale: String,
    /// `Intl.DateTimeFormat().resolvedOptions().timeZone`
    pub intl_timezone: String,
    /// `Intl.DateTimeFormat().format(...)` sample (what the page prints).
    #[serde(default)]
    pub intl_date_time_sample: String,
    /// `Intl.NumberFormat().format(...)` sample.
    #[serde(default)]
    pub intl_number_sample: String,
    /// ICE candidates (type + protocol + address).
    pub ice_candidates: Vec<BrowserIceCandidate>,
    /// `icegatheringstate` at completion.
    pub ice_gathering_state: String,
    /// Page-reported error, if any.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
pub struct BrowserIceCandidate {
    pub candidate: String,
    pub kind: String,
    pub protocol: String,
    pub address: String,
    pub class: Option<String>,
}

/// Parse the JSON blob the acceptance page writes into `#envbox-probe-result`.
pub fn parse_browser_ice_report(json: &str) -> Result<BrowserIceReport, String> {
    serde_json::from_str(json).map_err(|e| format!("browser report json: {e}"))
}

/// Extra assertions for a real-browser ICE/Intl report.
///
/// Strict: any non-relay candidate is a direct UDP path — public AND private/LAN
/// host candidates fail (spec: "Strict 无任何 direct UDP candidate").
/// ProxyOnly / Strict: no non-proxy (host/srflx/prflx) UDP candidates.
/// mDNS `.local` names are link-local host candidates (never "public").
pub fn evaluate_browser_ice(
    report: &BrowserIceReport,
    expected: PolicyEffective,
) -> Vec<String> {
    let mut violations = Vec::new();
    let strict = expected == PolicyEffective::Strict;
    let proxy_plus = matches!(
        expected,
        PolicyEffective::ProxyOnly | PolicyEffective::Strict
    );
    if !strict && !proxy_plus {
        return violations;
    }
    for c in &report.ice_candidates {
        let kind = c.kind.as_str();
        if kind == "relay" {
            continue;
        }
        let addr = c.address.as_str();
        let is_mdns = c.kind == "mdns"
            || c.candidate.contains(".local")
            || addr.ends_with(".local");
        if is_mdns {
            // mDNS host candidates are still direct host paths under Strict.
            if strict {
                violations.push(format!(
                    "strict-ice-direct: mDNS host candidate {} is a direct path",
                    if addr.is_empty() { &c.candidate } else { addr }
                ));
            }
            continue;
        }
        let _class = c.class.as_deref().unwrap_or("");
        if strict {
            // Any non-relay candidate is a direct UDP path.
            violations.push(format!(
                "strict-ice-direct: {kind} candidate {} is a direct path (policy = strict)",
                if addr.is_empty() { c.candidate.as_str() } else { addr }
            ));
            continue;
        }
        // ProxyOnly+: non-proxy UDP candidates (host/srflx/prflx) are not allowed
        // at all (disable_non_proxied_udp) - not merely "not public".
        if kind == "host" || kind == "srflx" || kind == "prflx" {
            violations.push(format!(
                "proxy-ice-non-proxied: {kind} candidate {} (policy = {})",
                if addr.is_empty() { c.candidate.as_str() } else { addr },
                expected.as_str()
            ));
        }
    }
    violations
}

/// Effective WebRTC policy as the process can observe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffective {
    /// No `ENVBOX_WEBRTC_POLICY` (or unparsable token) — not bound to a profile policy.
    Unbound,
    Host,
    PublicInterfaceOnly,
    ProxyOnly,
    Strict,
}

impl PolicyEffective {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyEffective::Unbound => "unbound",
            PolicyEffective::Host => "host",
            PolicyEffective::PublicInterfaceOnly => "public_interface_only",
            PolicyEffective::ProxyOnly => "proxy_only",
            PolicyEffective::Strict => "strict",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "unbound" => Some(PolicyEffective::Unbound),
            _ => WebRtcPolicy::parse(s).map(PolicyEffective::from),
        }
    }
}

impl From<WebRtcPolicy> for PolicyEffective {
    fn from(p: WebRtcPolicy) -> Self {
        match p {
            WebRtcPolicy::Host => PolicyEffective::Host,
            WebRtcPolicy::PublicInterfaceOnly => PolicyEffective::PublicInterfaceOnly,
            WebRtcPolicy::ProxyOnly => PolicyEffective::ProxyOnly,
            WebRtcPolicy::Strict => PolicyEffective::Strict,
        }
    }
}

/// Browser Policy environment artifacts (raw values, `null` when absent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PolicyEnv {
    #[serde(rename = "ENVBOX_WEBRTC_POLICY")]
    pub envbox_webrtc_policy: Option<String>,
    #[serde(rename = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")]
    pub webview2_additional_browser_arguments: Option<String>,
    #[serde(rename = "ENVBOX_PROFILE_ID")]
    pub envbox_profile_id: Option<String>,
    #[serde(rename = "ENVBOX_INSTANCE_ID")]
    pub envbox_instance_id: Option<String>,
}

/// Collect policy env entries via a lookup function (testable without std::env).
pub fn policy_env_from(lookup: impl Fn(&str) -> Option<String>) -> PolicyEnv {
    PolicyEnv {
        envbox_webrtc_policy: lookup("ENVBOX_WEBRTC_POLICY"),
        webview2_additional_browser_arguments: lookup("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"),
        envbox_profile_id: lookup("ENVBOX_PROFILE_ID"),
        envbox_instance_id: lookup("ENVBOX_INSTANCE_ID"),
    }
}

/// `unbound` when `ENVBOX_WEBRTC_POLICY` is missing or unparsable.
pub fn policy_effective_from(env: &PolicyEnv) -> PolicyEffective {
    let Some(token) = env.envbox_webrtc_policy.as_deref() else {
        return PolicyEffective::Unbound;
    };
    WebRtcPolicy::parse(token)
        .map(PolicyEffective::from)
        .unwrap_or(PolicyEffective::Unbound)
}

/// Chromium `--force-webrtc-ip-handling-policy` value for a policy.
/// `Host` / `Unbound` inject nothing (never overwrite an explicit user switch).
pub fn chromium_ip_handling_switch(policy: PolicyEffective) -> Option<&'static str> {
    match policy {
        PolicyEffective::PublicInterfaceOnly => Some("default_public_interface_only"),
        PolicyEffective::ProxyOnly | PolicyEffective::Strict => Some("disable_non_proxied_udp"),
        PolicyEffective::Host | PolicyEffective::Unbound => None,
    }
}

/// True when `args` already carries a `--force-webrtc-ip-handling-policy` switch.
pub fn has_webrtc_ip_handling_switch(args: &[String]) -> bool {
    args.iter()
        .any(|a| a.contains("--force-webrtc-ip-handling-policy"))
}

/// Locale compatibility: exact, subtag-boundary prefix, or one tag's subtags
/// are a subset of the other's (zh-CN vs zh-Hans-CN). Never primary-only
/// (zh-CN must not match zh-TW).
pub fn locale_matches(got: &str, want: &str) -> bool {
    let g = got.trim().to_ascii_lowercase();
    let w = want.trim().to_ascii_lowercase();
    if g == w {
        return true;
    }
    if g.is_empty() || w.is_empty() {
        return false;
    }
    if g.starts_with(&format!("{w}-")) || w.starts_with(&format!("{g}-")) {
        return true;
    }
    let gs: Vec<&str> = g.split('-').collect();
    let ws: Vec<&str> = w.split('-').collect();
    ws.iter().all(|s| gs.contains(s)) || gs.iter().all(|s| ws.contains(s))
}

/// Format-sample check for date/number printing style.
/// `en` uses 1,234,567.89 (dot decimal); several EU locales use 1.234.567,89.
pub fn intl_format_sample_mismatches(
    report: &BrowserIceReport,
    want_locale: &str,
) -> Vec<String> {
    let mut violations = Vec::new();
    let want = want_locale.trim().to_ascii_lowercase();
    let num = report.intl_number_sample.as_str();
    if !num.is_empty() && want.starts_with("en") {
        // Comma-as-decimal (e.g. 1234567,89) is not en style.
        if num.contains(',') && !num.contains('.') {
            violations.push(format!(
                "intl-number-sample: {num:?} does not look like en (expected 1,234,567.89) for {want_locale:?}"
            ));
        }
    }
    violations
}

/// Locale assertion helper: every populated Intl / navigator locale field must
/// agree with `want`.
pub fn intl_locale_mismatches(
    report: &BrowserIceReport,
    want: &str,
) -> Vec<String> {
    let mut violations = Vec::new();
    let mut fields: Vec<(&str, &str)> = vec![
        ("intl_date_time_locale", report.intl_date_time_locale.as_str()),
        ("intl_number_locale", report.intl_number_locale.as_str()),
        ("navigator_language", report.navigator_language.as_str()),
    ];
    if let Some(first) = report.navigator_languages.first() {
        fields.push(("navigator_languages[0]", first.as_str()));
    }
    for (name, got) in fields {
        if got.trim().is_empty() {
            continue;
        }
        if !locale_matches(got, want) {
            violations.push(format!(
                "intl-locale: {name}={got:?} does not match expected {want:?}"
            ));
        }
    }
    if !report.navigator_languages.is_empty() && want.trim().is_empty() {
        violations.push("intl-locale: expected empty but navigator.languages is set".into());
    }
    violations
}

/// One observed local unicast address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalAddress {
    pub address: String,
    pub family: IpFamily,
    pub class: IpClass,
}

impl LocalAddress {
    pub fn from_ip(ip: std::net::IpAddr) -> Self {
        let family = if ip.is_ipv4() {
            IpFamily::Ipv4
        } else {
            IpFamily::Ipv6
        };
        LocalAddress {
            address: ip.to_string(),
            family,
            class: classify_ip(ip),
        }
    }
}

/// One STUN Binding result (direct UDP, never proxied).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StunCandidate {
    pub server: String,
    pub transport: String,
    pub local: String,
    pub reflexive: String,
    pub reflexive_class: IpClass,
}

/// Policy expectations a browser under this policy must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Flags {
    /// ProxyOnly / Strict: Chromium `disable_non_proxied_udp`.
    pub expect_no_non_proxied_udp: bool,
    /// Strict: no direct UDP path at all (Network Guard).
    pub expect_no_direct_udp: bool,
}

impl Flags {
    pub fn for_policy(p: PolicyEffective) -> Self {
        Flags {
            expect_no_non_proxied_udp: matches!(
                p,
                PolicyEffective::ProxyOnly | PolicyEffective::Strict
            ),
            expect_no_direct_udp: matches!(p, PolicyEffective::Strict),
        }
    }
}

/// Machine-readable probe report (JSON on stdout).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub policy_env: PolicyEnv,
    pub policy_effective: PolicyEffective,
    pub local_addresses: Vec<LocalAddress>,
    pub stun_candidates: Vec<StunCandidate>,
    pub stun_error: Option<String>,
    pub flags: Flags,
}

/// Build a report from already-collected pieces (pure; used by main and tests).
pub fn build_report(
    policy_env: PolicyEnv,
    local_addresses: Vec<LocalAddress>,
    stun_candidates: Vec<StunCandidate>,
    stun_error: Option<String>,
) -> Report {
    let policy_effective = policy_effective_from(&policy_env);
    let flags = Flags::for_policy(policy_effective);
    Report {
        policy_env,
        policy_effective,
        local_addresses,
        stun_candidates,
        stun_error,
        flags,
    }
}

/// True when the WebView2 argument string carries `disable_non_proxied_udp`.
pub fn webview2_has_disable_non_proxied_udp(args: Option<&str>) -> bool {
    args.map(|s| s.to_ascii_lowercase().contains("disable_non_proxied_udp"))
        .unwrap_or(false)
}

/// Conservative `--assert` checks. Returns violation messages (empty = pass).
///
/// `expected` is the policy under acceptance (`--expect-policy`, defaulting to
/// the observed `policy_effective`). Checks are deliberately narrow:
///
/// 1. **policy-not-applied** (expected ProxyOnly/Strict): fail when any `public`
///    local address exists while `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` lacks
///    `disable_non_proxied_udp` AND `ENVBOX_WEBRTC_POLICY` is missing.
/// 2. **strict-direct-udp** (expected Strict, STUN ran): fail when STUN returns
///    a public server-reflexive candidate. This probe never proxies its UDP, so
///    a public reflexive address is by definition a non-proxy direct path.
pub fn evaluate_assertions(report: &Report, expected: PolicyEffective) -> Vec<String> {
    let mut violations = Vec::new();

    let policy_sensitive = matches!(
        expected,
        PolicyEffective::ProxyOnly | PolicyEffective::Strict
    );
    if policy_sensitive {
        let has_public = report
            .local_addresses
            .iter()
            .any(|a| a.class == IpClass::Public);
        let webview2_ok = webview2_has_disable_non_proxied_udp(
            report
                .policy_env
                .webview2_additional_browser_arguments
                .as_deref(),
        );
        let webrtc_env_missing = report.policy_env.envbox_webrtc_policy.is_none();
        if has_public && !webview2_ok && webrtc_env_missing {
            violations.push(
                "policy-not-applied: public local address present while \
                 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS lacks disable_non_proxied_udp \
                 and ENVBOX_WEBRTC_POLICY is missing"
                    .to_string(),
            );
        }
    }

    if expected == PolicyEffective::Strict {
        for c in &report.stun_candidates {
            if c.reflexive_class == IpClass::Public {
                violations.push(format!(
                    "strict-direct-udp: non-proxy public reflexive {} via {}",
                    c.reflexive, c.server
                ));
            }
        }
    }

    violations
}

impl Report {
    /// Human-readable rendering (stderr by default, stdout with `--text`).
    pub fn render_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "=== BROWSER PROBE ===");
        let _ = writeln!(out, "policy_effective: {}", self.policy_effective.as_str());
        let pe = &self.policy_env;
        let _ = writeln!(
            out,
            "ENVBOX_WEBRTC_POLICY: {}",
            pe.envbox_webrtc_policy.as_deref().unwrap_or("<absent>")
        );
        let _ = writeln!(
            out,
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: {}",
            pe.webview2_additional_browser_arguments
                .as_deref()
                .unwrap_or("<absent>")
        );
        let _ = writeln!(
            out,
            "ENVBOX_PROFILE_ID: {}",
            pe.envbox_profile_id.as_deref().unwrap_or("<absent>")
        );
        let _ = writeln!(
            out,
            "ENVBOX_INSTANCE_ID: {}",
            pe.envbox_instance_id.as_deref().unwrap_or("<absent>")
        );
        let _ = writeln!(out, "=== LOCAL ADDRESSES ===");
        for a in &self.local_addresses {
            let _ = writeln!(
                out,
                "{} {} {}",
                a.address,
                match a.family {
                    IpFamily::Ipv4 => "ipv4",
                    IpFamily::Ipv6 => "ipv6",
                },
                a.class.as_str()
            );
        }
        let _ = writeln!(out, "=== STUN ===");
        if let Some(err) = &self.stun_error {
            let _ = writeln!(out, "stun_error: {err}");
        }
        for c in &self.stun_candidates {
            let _ = writeln!(
                out,
                "{} {} local={} reflexive={} class={}",
                c.server,
                c.transport,
                c.local,
                c.reflexive,
                c.reflexive_class.as_str()
            );
        }
        if self.stun_candidates.is_empty() && self.stun_error.is_none() {
            let _ = writeln!(out, "<not requested>");
        }
        let _ = writeln!(out, "=== FLAGS ===");
        let _ = writeln!(
            out,
            "expect_no_non_proxied_udp: {}",
            self.flags.expect_no_non_proxied_udp
        );
        let _ = writeln!(out, "expect_no_direct_udp: {}", self.flags.expect_no_direct_udp);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr, IpAddr};

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("test ip")
    }

    #[test]
    fn classify_ipv4() {
        assert_eq!(classify_ip(ip("127.0.0.1")), IpClass::Loopback);
        assert_eq!(classify_ip(ip("127.255.0.1")), IpClass::Loopback);
        assert_eq!(classify_ip(ip("169.254.10.10")), IpClass::LinkLocal);
        assert_eq!(classify_ip(ip("10.0.0.1")), IpClass::Private);
        assert_eq!(classify_ip(ip("172.16.0.1")), IpClass::Private);
        assert_eq!(classify_ip(ip("172.31.255.255")), IpClass::Private);
        assert_eq!(classify_ip(ip("172.32.0.1")), IpClass::Public);
        assert_eq!(classify_ip(ip("192.168.1.1")), IpClass::Private);
        assert_eq!(classify_ip(ip("100.64.0.1")), IpClass::Private);
        assert_eq!(classify_ip(ip("100.128.0.1")), IpClass::Public);
        assert_eq!(classify_ip(ip("8.8.8.8")), IpClass::Public);
        assert_eq!(classify_ip(ip("203.0.113.5")), IpClass::Public);
    }

    #[test]
    fn classify_ipv6() {
        assert_eq!(classify_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)), IpClass::Loopback);
        assert_eq!(classify_ip(ip("fe80::1")), IpClass::LinkLocal);
        assert_eq!(classify_ip(ip("febf::1")), IpClass::LinkLocal);
        assert_eq!(classify_ip(ip("fc00::1")), IpClass::Private);
        assert_eq!(classify_ip(ip("fd12:3456::1")), IpClass::Private);
        assert_eq!(classify_ip(ip("2001:db8::1")), IpClass::Public);
        assert_eq!(classify_ip(ip("::ffff:10.0.0.1")), IpClass::Private);
        assert_eq!(classify_ip(ip("::ffff:8.8.8.8")), IpClass::Public);
    }

    #[test]
    fn classify_helpers_cover_edge_boundaries() {
        // v4 boundary just inside/outside 172.16/12
        assert_eq!(classify_v4([172, 15, 255, 255]), IpClass::Public);
        assert_eq!(classify_v4([172, 16, 0, 0]), IpClass::Private);
        assert_eq!(classify_v4([172, 31, 255, 255]), IpClass::Private);
        assert_eq!(classify_v4([172, 32, 0, 0]), IpClass::Public);
        // fe80::/10 upper edge
        assert_eq!(
            classify_v6([0xfe, 0xbf, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
            IpClass::LinkLocal
        );
        assert_eq!(
            classify_v6([0xfe, 0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
            IpClass::Public
        );
        let _ = Ipv4Addr::UNSPECIFIED;
    }

    #[test]
    fn stun_xor_mapped_ipv6_full_mask() {
        // RFC 5389 §15.2: IPv6 XOR is magic || txid over all 16 address bytes.
        let txid: [u8; 12] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        ];
        let ip = std::net::Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1);
        let raw = ip.octets();
        let mut enc = [0u8; 16];
        for i in 0..4 {
            enc[i] = raw[i] ^ STUN_MAGIC[i];
        }
        for i in 0..12 {
            enc[4 + i] = raw[4 + i] ^ txid[i];
        }
        let mut body = vec![0u8, 0x02, 0x00, 0x00];
        // port 3478 xor 0x2112 = 0x146a
        let port: u16 = 3478 ^ 0x2112;
        body[2..4].copy_from_slice(&port.to_be_bytes());
        body.extend_from_slice(&enc);
        let (got_ip, got_port) =
            decode_mapped_address(0x0020, &body, &txid).expect("decode ipv6");
        assert_eq!(got_ip, std::net::IpAddr::V6(ip));
        assert_eq!(got_port, 3478);
        // Naive magic-only XOR must NOT match (the old bug).
        let mut naive = raw;
        for i in (0..16).step_by(4) {
            naive[i] ^= 0x21;
            naive[i + 1] ^= 0x12;
            naive[i + 2] ^= 0xa4;
            naive[i + 3] ^= 0x42;
        }
        assert_ne!(naive, enc);
    }

    #[test]
    fn stun_xor_mapped_ipv4_magic_only() {
        let txid = [0u8; 12];
        let mut body = vec![0u8, 0x01, 0x00, 0x00];
        let port_x: u16 = 3478 ^ 0x2112;
        body[2..4].copy_from_slice(&port_x.to_be_bytes());
        let a: u32 = 0x2112a442 ^ u32::from_be_bytes([203, 0, 113, 5]);
        body.extend_from_slice(&a.to_be_bytes());
        let (ip, port) = decode_mapped_address(0x0020, &body, &txid).expect("v4");
        assert_eq!(ip, std::net::IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 5)));
        assert_eq!(port, 3478);
    }

    #[test]
    fn browser_ice_strict_flags_public_host() {
        let report = BrowserIceReport {
            ice_candidates: vec![BrowserIceCandidate {
                candidate: "candidate:1 1 udp 2122260223 203.0.113.5 54321 typ host"
                    .into(),
                kind: "host".into(),
                protocol: "udp".into(),
                address: "203.0.113.5".into(),
                class: Some("public".into()),
            }],
            ..Default::default()
        };
        let v = evaluate_browser_ice(&report, PolicyEffective::Strict);
        assert_eq!(v.len(), 1);
        // Private LAN host is still a direct path under Strict.
        let lan = BrowserIceReport {
            ice_candidates: vec![BrowserIceCandidate {
                candidate: "host".into(),
                kind: "host".into(),
                protocol: "udp".into(),
                address: "192.168.1.10".into(),
                class: Some("private".into()),
            }],
            ..Default::default()
        };
        assert_eq!(evaluate_browser_ice(&lan, PolicyEffective::Strict).len(), 1);
        // mDNS is a direct host path under Strict too.
        let mdns = BrowserIceReport {
            ice_candidates: vec![BrowserIceCandidate {
                candidate: "host .local".into(),
                kind: "mdns".into(),
                protocol: "udp".into(),
                address: "abc.local".into(),
                class: Some("link_local".into()),
            }],
            ..Default::default()
        };
        assert_eq!(evaluate_browser_ice(&mdns, PolicyEffective::Strict).len(), 1);
        let ok = BrowserIceReport {
            ice_candidates: vec![BrowserIceCandidate {
                candidate: "relay".into(),
                kind: "relay".into(),
                protocol: "udp".into(),
                address: "198.51.100.1".into(),
                class: Some("public".into()),
            }],
            ..Default::default()
        };
        assert!(evaluate_browser_ice(&ok, PolicyEffective::Strict).is_empty());
        // ProxyOnly: any non-proxy UDP candidate fails, even private.
        assert_eq!(
            evaluate_browser_ice(&lan, PolicyEffective::ProxyOnly).len(),
            1
        );
    }

    #[test]
    fn policy_from_env_lookup() {
        let none = policy_env_from(|_| None);
        assert_eq!(none.envbox_webrtc_policy, None);
        assert_eq!(policy_effective_from(&none), PolicyEffective::Unbound);

        let bound = policy_env_from(|k| match k {
            "ENVBOX_WEBRTC_POLICY" => Some("proxy_only".into()),
            "ENVBOX_PROFILE_ID" => Some("pid".into()),
            _ => None,
        });
        assert_eq!(policy_effective_from(&bound), PolicyEffective::ProxyOnly);

        let garbage = policy_env_from(|k| match k {
            "ENVBOX_WEBRTC_POLICY" => Some("nope".into()),
            _ => None,
        });
        assert_eq!(policy_effective_from(&garbage), PolicyEffective::Unbound);

        let strict = policy_env_from(|k| match k {
            "ENVBOX_WEBRTC_POLICY" => Some("Strict".into()),
            _ => None,
        });
        assert_eq!(policy_effective_from(&strict), PolicyEffective::Strict);

        // PolicyEffective::parse accepts unbound + WebRtcPolicy tokens.
        assert_eq!(
            PolicyEffective::parse("public_interface_only"),
            Some(PolicyEffective::PublicInterfaceOnly)
        );
        assert_eq!(PolicyEffective::parse("unbound"), Some(PolicyEffective::Unbound));
        assert_eq!(PolicyEffective::parse("bogus"), None);
    }

    #[test]
    fn flags_follow_policy() {
        assert_eq!(
            Flags::for_policy(PolicyEffective::ProxyOnly),
            Flags {
                expect_no_non_proxied_udp: true,
                expect_no_direct_udp: false
            }
        );
        assert_eq!(
            Flags::for_policy(PolicyEffective::Strict),
            Flags {
                expect_no_non_proxied_udp: true,
                expect_no_direct_udp: true
            }
        );
        assert_eq!(
            Flags::for_policy(PolicyEffective::Host),
            Flags {
                expect_no_non_proxied_udp: false,
                expect_no_direct_udp: false
            }
        );
        assert_eq!(
            Flags::for_policy(PolicyEffective::Unbound),
            Flags {
                expect_no_non_proxied_udp: false,
                expect_no_direct_udp: false
            }
        );
    }

    fn sample_report() -> Report {
        build_report(
            PolicyEnv {
                envbox_webrtc_policy: Some("proxy_only".into()),
                webview2_additional_browser_arguments: Some(
                    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
                ),
                envbox_profile_id: Some("prof-1".into()),
                envbox_instance_id: Some("inst-1".into()),
            },
            vec![LocalAddress::from_ip(ip("192.168.1.5"))],
            vec![],
            None,
        )
    }

    #[test]
    fn json_report_shape() {
        let report = sample_report();
        let v = serde_json::to_value(&report).expect("serialize report");
        assert_eq!(v["policy_effective"], "proxy_only");
        assert_eq!(v["policy_env"]["ENVBOX_WEBRTC_POLICY"], "proxy_only");
        assert_eq!(
            v["policy_env"]["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"],
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"
        );
        assert_eq!(v["policy_env"]["ENVBOX_PROFILE_ID"], "prof-1");
        assert_eq!(v["policy_env"]["ENVBOX_INSTANCE_ID"], "inst-1");
        assert!(v["local_addresses"].is_array());
        assert_eq!(v["local_addresses"][0]["address"], "192.168.1.5");
        assert_eq!(v["local_addresses"][0]["family"], "ipv4");
        assert_eq!(v["local_addresses"][0]["class"], "private");
        assert!(v["stun_candidates"].is_array());
        assert!(v["stun_candidates"].as_array().unwrap().is_empty());
        assert!(v["stun_error"].is_null());
        assert_eq!(v["flags"]["expect_no_non_proxied_udp"], true);
        assert_eq!(v["flags"]["expect_no_direct_udp"], false);
    }

    #[test]
    fn json_absent_policy_env_is_null() {
        let report = build_report(PolicyEnv::default(), vec![], vec![], None);
        let v = serde_json::to_value(&report).expect("serialize");
        assert!(v["policy_env"]["ENVBOX_WEBRTC_POLICY"].is_null());
        assert_eq!(v["policy_effective"], "unbound");
    }

    #[test]
    fn assert_policy_not_applied_fires_only_with_public_addr() {
        let mut report = sample_report();
        // Simulate a failed policy apply: no env artifacts at all.
        report.policy_env = PolicyEnv::default();
        report.local_addresses = vec![LocalAddress::from_ip(ip("203.0.113.5"))];
        let v = evaluate_assertions(&report, PolicyEffective::ProxyOnly);
        assert_eq!(v.len(), 1, "expected one violation, got {v:?}");
        assert!(v[0].starts_with("policy-not-applied"), "got {}", v[0]);

        // Same gap but only private addresses — conservative pass.
        report.local_addresses = vec![LocalAddress::from_ip(ip("192.168.1.5"))];
        assert!(evaluate_assertions(&report, PolicyEffective::ProxyOnly).is_empty());

        // Host / unbound expectations never trigger the policy-not-applied check.
        report.local_addresses = vec![LocalAddress::from_ip(ip("203.0.113.5"))];
        assert!(evaluate_assertions(&report, PolicyEffective::Host).is_empty());
        assert!(evaluate_assertions(&report, PolicyEffective::Unbound).is_empty());
    }

    #[test]
    fn assert_policy_not_applied_passes_when_switch_applied() {
        let mut report = sample_report();
        report.policy_env.envbox_webrtc_policy = None;
        report.local_addresses = vec![LocalAddress::from_ip(ip("203.0.113.5"))];
        // WEBVIEW2 switch present — even with ENVBOX_WEBRTC_POLICY missing, the
        // documented condition requires BOTH gaps.
        assert!(evaluate_assertions(&report, PolicyEffective::ProxyOnly).is_empty());

        // ENVBOX_WEBRTC_POLICY present (policy applied) — also passes.
        let mut report2 = sample_report();
        report2.policy_env.webview2_additional_browser_arguments = None;
        report2.local_addresses = vec![LocalAddress::from_ip(ip("203.0.113.5"))];
        assert!(evaluate_assertions(&report2, PolicyEffective::ProxyOnly).is_empty());
    }

    #[test]
    fn assert_strict_fails_on_public_reflexive() {
        let mut report = sample_report();
        report.stun_candidates = vec![StunCandidate {
            server: "stun.example:3478".into(),
            transport: "udp".into(),
            local: "192.168.1.5:50000".into(),
            reflexive: "198.51.100.9:50000".into(),
            reflexive_class: IpClass::Public,
        }];
        let v = evaluate_assertions(&report, PolicyEffective::Strict);
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(v[0].starts_with("strict-direct-udp"), "got {}", v[0]);

        // Private reflexive is not a direct-public path — pass.
        report.stun_candidates[0].reflexive = "10.0.0.2:50000".into();
        report.stun_candidates[0].reflexive_class = IpClass::Private;
        assert!(evaluate_assertions(&report, PolicyEffective::Strict).is_empty());

        // ProxyOnly does not assert on STUN reflexives.
        report.stun_candidates[0].reflexive_class = IpClass::Public;
        assert!(evaluate_assertions(&report, PolicyEffective::ProxyOnly).is_empty());
    }

    #[test]
    fn webview2_switch_detection() {
        assert!(!webview2_has_disable_non_proxied_udp(None));
        assert!(!webview2_has_disable_non_proxied_udp(Some("--foo")));
        assert!(webview2_has_disable_non_proxied_udp(Some(
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"
        )));
        assert!(webview2_has_disable_non_proxied_udp(Some(
            "--FOO --force-webrtc-ip-handling-policy=DISABLE_NON_PROXIED_UDP"
        )));
    }

    #[test]
    fn chromium_switch_mapping() {
        assert_eq!(chromium_ip_handling_switch(PolicyEffective::Host), None);
        assert_eq!(chromium_ip_handling_switch(PolicyEffective::Unbound), None);
        assert_eq!(
            chromium_ip_handling_switch(PolicyEffective::PublicInterfaceOnly),
            Some("default_public_interface_only")
        );
        assert_eq!(
            chromium_ip_handling_switch(PolicyEffective::ProxyOnly),
            Some("disable_non_proxied_udp")
        );
        assert_eq!(
            chromium_ip_handling_switch(PolicyEffective::Strict),
            Some("disable_non_proxied_udp")
        );
        assert!(has_webrtc_ip_handling_switch(&[
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into()
        ]));
        assert!(!has_webrtc_ip_handling_switch(&["--foo".into()]));
    }

    #[test]
    fn intl_locale_fields_all_participate() {
        let mut r = BrowserIceReport {
            navigator_language: "zh-CN".into(),
            navigator_languages: vec!["zh-CN".into(), "en".into()],
            intl_date_time_locale: "zh-CN".into(),
            intl_number_locale: "zh-CN".into(),
            intl_timezone: "Asia/Shanghai".into(),
            ..Default::default()
        };
        assert!(intl_locale_mismatches(&r, "zh-CN").is_empty());

        // NumberFormat mismatch alone is a violation (not OR-ed away).
        r.intl_number_locale = "en-US".into();
        let v = intl_locale_mismatches(&r, "zh-CN");
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(v[0].contains("intl_number_locale"));

        // navigator_languages[0] mismatch is a violation.
        r.intl_number_locale = "zh-CN".into();
        r.navigator_languages = vec!["en-US".into()];
        let v = intl_locale_mismatches(&r, "zh-CN");
        assert_eq!(v.len(), 1, "got {v:?}");
        assert!(v[0].contains("navigator_languages"));

        // Prefix tolerance: zh-Hans-CN matches expected zh-CN.
        r.navigator_languages = vec!["zh-Hans-CN".into()];
        r.intl_date_time_locale = "zh-Hans-CN".into();
        assert!(intl_locale_mismatches(&r, "zh-CN").is_empty());
    }
}
