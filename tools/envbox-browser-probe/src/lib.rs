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
}
