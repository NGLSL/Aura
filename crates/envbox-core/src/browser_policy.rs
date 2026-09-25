//! Browser / Network Guard policy (WebRTC Privacy).
//!
//! Separate from Geo/Locale hooks: this module only decides browser engine
//! classification and policy artifacts (Chromium switch / WebView2 args).
//! Network enforcement for `Strict` lives in Runtime / Network Guard.

use serde::{Deserialize, Serialize};

/// WebRTC privacy policy for one Environment Profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebRtcPolicy {
    /// Do not alter browser WebRTC policy.
    #[default]
    Host,
    /// Hide local/private interfaces; still allow normal public UDP.
    PublicInterfaceOnly,
    /// Chromium `disable_non_proxied_udp` (proxy-only UDP).
    ProxyOnly,
    /// ProxyOnly + session Network Guard (no direct UDP fallback).
    Strict,
}

impl WebRtcPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            WebRtcPolicy::Host => "host",
            WebRtcPolicy::PublicInterfaceOnly => "public_interface_only",
            WebRtcPolicy::ProxyOnly => "proxy_only",
            WebRtcPolicy::Strict => "strict",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "host" => Some(WebRtcPolicy::Host),
            "public_interface_only" | "public-interface-only" => {
                Some(WebRtcPolicy::PublicInterfaceOnly)
            }
            "proxy_only" | "proxy-only" | "disable_non_proxied_udp" => Some(WebRtcPolicy::ProxyOnly),
            "strict" => Some(WebRtcPolicy::Strict),
            _ => None,
        }
    }

    /// Chromium `--force-webrtc-ip-handling-policy` value.
    pub fn chromium_ip_handling_policy(self) -> Option<&'static str> {
        match self {
            WebRtcPolicy::Host => None,
            WebRtcPolicy::PublicInterfaceOnly => Some("default_public_interface_only"),
            WebRtcPolicy::ProxyOnly | WebRtcPolicy::Strict => Some("disable_non_proxied_udp"),
        }
    }

    /// True when Network Guard must enforce UDP constraints.
    pub fn requires_network_guard(self) -> bool {
        matches!(self, WebRtcPolicy::Strict)
    }

    /// Browser-layer guarantee level (parallel to IsolationGuarantee).
    pub fn browser_guarantee(self) -> BrowserGuarantee {
        match self {
            WebRtcPolicy::Host | WebRtcPolicy::PublicInterfaceOnly | WebRtcPolicy::ProxyOnly => {
                BrowserGuarantee::PolicyOnly
            }
            WebRtcPolicy::Strict => BrowserGuarantee::NetworkEnforced,
        }
    }
}

/// What Browser / Network Guard actually delivers for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserGuarantee {
    /// Chromium/WebView2 policy artifacts only (Balanced).
    PolicyOnly,
    /// Policy + session Network Guard (Strict). Requires guard availability.
    NetworkEnforced,
}

/// Browser privacy settings on a Profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrowserPrivacyProfile {
    #[serde(default)]
    pub webrtc: WebRtcPolicy,
}

/// Explicit browser engine classification (never bare substring match).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngine {
    Chromium,
    Edge,
    WebView2,
    Electron,
    Unknown,
}

impl BrowserEngine {
    /// Classify from executable path / image name (case-insensitive).
    pub fn from_image(image: &str) -> Self {
        let name = image
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(image)
            .to_ascii_lowercase();
        // WebView2 host first (msedgewebview2.exe).
        if name == "msedgewebview2.exe" || name.starts_with("msedgewebview2") {
            return BrowserEngine::WebView2;
        }
        if name == "electron.exe" || name.ends_with("electron.exe") {
            return BrowserEngine::Electron;
        }
        if name == "msedge.exe" {
            return BrowserEngine::Edge;
        }
        if name == "chrome.exe"
            || name == "chromium.exe"
            || name == "chrome_proxy.exe"
            || name == "googlechromeproxy.exe"
        {
            return BrowserEngine::Chromium;
        }
        // Chromium-based helpers spawned beside a known engine (best-effort).
        if name == "msedgewebview2.exe" {
            return BrowserEngine::WebView2;
        }
        BrowserEngine::Unknown
    }

    pub fn is_browser(self) -> bool {
        !matches!(self, BrowserEngine::Unknown)
    }
}

/// How a policy artifact was applied to a command line / env.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyApply {
    /// No change (Host / unknown / no artifact).
    Unchanged,
    /// Switch/variable inserted.
    Inserted,
    /// Existing conflicting artifact rewritten to Profile policy.
    Rewritten,
}

impl PolicyApply {
    /// Stable Audit API names (JSONL `api` field). C++ Runtime uses the same.
    pub fn audit_api(self) -> &'static str {
        match self {
            PolicyApply::Unchanged => "BrowserPolicyUnchanged",
            PolicyApply::Inserted => "BrowserPolicyApplied",
            PolicyApply::Rewritten => "BrowserPolicyConflictResolved",
        }
    }
}

/// Stable Audit API for Network Guard UDP deny.
pub const AUDIT_API_NETWORK_UDP_DENY: &str = "NetworkGuardUdpDeny";

/// Result of ensuring the Chromium WebRTC switch on a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLinePolicy {
    pub command_line: String,
    pub apply: PolicyApply,
}

/// Apply Chromium WebRTC switch to an argv vector (CreateProcess form).
/// Host / no artifact → unchanged. Existing switch rewritten; else appended.
pub fn ensure_chromium_webrtc_argv(args: &mut Vec<String>, policy: WebRtcPolicy) -> PolicyApply {
    let Some(value) = policy.chromium_ip_handling_policy() else {
        return PolicyApply::Unchanged;
    };
    let switch_val = format!("{CHROMIUM_SWITCH}={value}");
    let needle = CHROMIUM_SWITCH.to_ascii_lowercase();
    for arg in args.iter_mut() {
        let lower = arg.to_ascii_lowercase();
        if let Some(pos) = lower.find(&needle) {
            // Rewrite this argv token to the Profile policy value.
            *arg = switch_val;
            let _ = pos;
            return PolicyApply::Rewritten;
        }
    }
    args.push(switch_val);
    PolicyApply::Inserted
}

const CHROMIUM_SWITCH: &str = "--force-webrtc-ip-handling-policy";
const WEBVIEW2_ENV: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";

/// Ensure Chromium WebRTC IP policy switch on a command line.
///
/// - `Host` → unchanged
/// - existing switch → rewritten to Profile policy (never duplicated)
/// - missing → inserted
pub fn ensure_chromium_webrtc_switch(command_line: &str, policy: WebRtcPolicy) -> CommandLinePolicy {
    let Some(value) = policy.chromium_ip_handling_policy() else {
        return CommandLinePolicy {
            command_line: command_line.to_string(),
            apply: PolicyApply::Unchanged,
        };
    };
    let switch_val = format!("{CHROMIUM_SWITCH}={value}");
    let lower = command_line.to_ascii_lowercase();
    let needle = CHROMIUM_SWITCH.to_ascii_lowercase();

    if let Some(pos) = lower.find(&needle) {
        // Rewrite existing switch value in place (one occurrence expected).
        let after = pos + CHROMIUM_SWITCH.len();
        let rest = &command_line[after..];
        let bytes = rest.as_bytes();
        let mut end = 0usize;
        if bytes.first() == Some(&b'=') {
            end = 1;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
        } else if bytes.first().map(|c| c.is_ascii_whitespace()).unwrap_or(false) {
            // `--switch value` form: consume whitespace + value token.
            while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
        }
        let mut out = String::with_capacity(command_line.len() + 32);
        out.push_str(&command_line[..pos]);
        out.push_str(&switch_val);
        out.push_str(&command_line[after + end..]);
        return CommandLinePolicy {
            command_line: out,
            apply: PolicyApply::Rewritten,
        };
    }

    let mut out = command_line.to_string();
    if !out.trim().is_empty() {
        out.push(' ');
    }
    out.push_str(&switch_val);
    CommandLinePolicy {
        command_line: out,
        apply: PolicyApply::Inserted,
    }
}

/// Ensure `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` carries the Chromium switch.
pub fn ensure_webview2_arguments(
    existing: Option<&str>,
    policy: WebRtcPolicy,
) -> (Option<String>, PolicyApply) {
    let Some(value) = policy.chromium_ip_handling_policy() else {
        return (existing.map(|s| s.to_string()), PolicyApply::Unchanged);
    };
    let switch_val = format!("{CHROMIUM_SWITCH}={value}");
    let Some(prev) = existing else {
        return (
            Some(switch_val),
            PolicyApply::Inserted,
        );
    };
    let lower = prev.to_ascii_lowercase();
    if lower.contains(&CHROMIUM_SWITCH.to_ascii_lowercase()) {
        let ensured = ensure_chromium_webrtc_switch(prev, policy);
        return (
            Some(ensured.command_line),
            if ensured.apply == PolicyApply::Unchanged {
                PolicyApply::Rewritten
            } else {
                ensured.apply
            },
        );
    }
    let mut out = prev.trim().to_string();
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(&switch_val);
    (Some(out), PolicyApply::Inserted)
}

/// Decide policy artifacts for a CreateProcess image + command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserChildPolicy {
    pub engine: BrowserEngine,
    pub command_line: String,
    pub webview2_additional_args: Option<String>,
    pub apply: PolicyApply,
}

/// Apply Browser Policy for a child create (Runtime Child Guard).
pub fn plan_child_policy(
    image: &str,
    command_line: &str,
    policy: WebRtcPolicy,
    existing_webview2_args: Option<&str>,
) -> BrowserChildPolicy {
    let engine = BrowserEngine::from_image(image);
    if policy == WebRtcPolicy::Host {
        return BrowserChildPolicy {
            engine,
            command_line: command_line.to_string(),
            webview2_additional_args: existing_webview2_args.map(|s| s.to_string()),
            apply: PolicyApply::Unchanged,
        };
    }
    match engine {
        BrowserEngine::Unknown => BrowserChildPolicy {
            engine,
            command_line: command_line.to_string(),
            webview2_additional_args: existing_webview2_args.map(|s| s.to_string()),
            apply: PolicyApply::Unchanged,
        },
        BrowserEngine::WebView2 => {
            let (args, apply) = ensure_webview2_arguments(existing_webview2_args, policy);
            BrowserChildPolicy {
                engine,
                command_line: command_line.to_string(),
                webview2_additional_args: args,
                apply,
            }
        }
        BrowserEngine::Chromium | BrowserEngine::Edge | BrowserEngine::Electron => {
            let cmd = ensure_chromium_webrtc_switch(command_line, policy);
            BrowserChildPolicy {
                engine,
                command_line: cmd.command_line,
                webview2_additional_args: existing_webview2_args.map(|s| s.to_string()),
                apply: cmd.apply,
            }
        }
    }
}

/// Environment contributions for Browser Policy (WebView2 + Runtime fallback).
pub fn browser_env_entries(policy: WebRtcPolicy) -> Vec<(String, String)> {
    let mut out = vec![("ENVBOX_WEBRTC_POLICY".into(), policy.as_str().to_string())];
    if let Some(value) = policy.chromium_ip_handling_policy() {
        if let Some(args) = ensure_webview2_arguments(None, policy).0 {
            let _ = value;
            out.push((WEBVIEW2_ENV.into(), args));
        }
    }
    out
}

/// Network Guard availability for Strict startup (ticket 56).
///
/// Process-tree UDP enforcement lives in Runtime WinSock hooks (`hooks_network`).
/// Strict may start only when that enforcement is in the process tree.
/// Startup Fail Policy: refuse Strict rather than silently run as Balanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetworkGuardCapability {
    /// Runtime can deny non-loopback UDP from injected processes.
    pub session_udp_enforced: bool,
}

impl NetworkGuardCapability {
    /// No UDP enforcement (Browser Policy only). Strict is refused.
    pub fn browser_policy_only() -> Self {
        Self {
            session_udp_enforced: false,
        }
    }

    /// Runtime WinSock Network Guard is present (hooks_network.cpp).
    pub fn runtime_udp_enforced() -> Self {
        Self {
            session_udp_enforced: true,
        }
    }

    /// Startup check for a profile policy under this capability.
    /// `Err` means Startup Fail Policy — never launch a lie.
    pub fn check_policy(
        &self,
        policy: WebRtcPolicy,
    ) -> Result<BrowserGuarantee, String> {
        if policy.requires_network_guard() && !self.session_udp_enforced {
            return Err(format!(
                "WebRtcPolicy::strict requires Network Guard (session UDP deny), \
                 which is not available; refusing to start (Startup Fail Policy). \
                 Use proxy_only for Browser-Policy-only protection."
            ));
        }
        Ok(policy.browser_guarantee())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_parse_and_str() {
        assert_eq!(WebRtcPolicy::parse("proxy_only"), Some(WebRtcPolicy::ProxyOnly));
        assert_eq!(WebRtcPolicy::parse("Strict"), Some(WebRtcPolicy::Strict));
        assert_eq!(WebRtcPolicy::Strict.as_str(), "strict");
        assert_eq!(WebRtcPolicy::parse("nope"), None);
    }

    #[test]
    fn chromium_mapping() {
        assert_eq!(WebRtcPolicy::Host.chromium_ip_handling_policy(), None);
        assert_eq!(
            WebRtcPolicy::PublicInterfaceOnly.chromium_ip_handling_policy(),
            Some("default_public_interface_only")
        );
        assert_eq!(
            WebRtcPolicy::ProxyOnly.chromium_ip_handling_policy(),
            Some("disable_non_proxied_udp")
        );
        assert_eq!(
            WebRtcPolicy::Strict.chromium_ip_handling_policy(),
            Some("disable_non_proxied_udp")
        );
        assert!(WebRtcPolicy::Strict.requires_network_guard());
        assert!(!WebRtcPolicy::ProxyOnly.requires_network_guard());
    }

    #[test]
    fn classify_engines() {
        assert_eq!(
            BrowserEngine::from_image(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            BrowserEngine::Chromium
        );
        assert_eq!(
            BrowserEngine::from_image("msedgewebview2.exe"),
            BrowserEngine::WebView2
        );
        assert_eq!(BrowserEngine::from_image("electron.exe"), BrowserEngine::Electron);
        assert_eq!(BrowserEngine::from_image("msedge.exe"), BrowserEngine::Edge);
        assert_eq!(
            BrowserEngine::from_image(r"C:\foo\ChatGPT.exe"),
            BrowserEngine::Unknown
        );
    }

    #[test]
    fn host_policy_leaves_command_line() {
        let r = ensure_chromium_webrtc_switch("chrome.exe --foo", WebRtcPolicy::Host);
        assert_eq!(r.apply, PolicyApply::Unchanged);
        assert_eq!(r.command_line, "chrome.exe --foo");
    }

    #[test]
    fn insert_switch_when_missing() {
        let r = ensure_chromium_webrtc_switch("chrome.exe", WebRtcPolicy::ProxyOnly);
        assert_eq!(r.apply, PolicyApply::Inserted);
        assert!(r
            .command_line
            .contains("--force-webrtc-ip-handling-policy=disable_non_proxied_udp"));
        assert_eq!(
            r.command_line
                .matches("--force-webrtc-ip-handling-policy")
                .count(),
            1
        );
    }

    #[test]
    fn rewrite_conflicting_switch_no_duplicate() {
        let base = "chrome.exe --force-webrtc-ip-handling-policy=default --foo";
        let r = ensure_chromium_webrtc_switch(base, WebRtcPolicy::Strict);
        assert_eq!(r.apply, PolicyApply::Rewritten);
        assert!(r.command_line.contains("disable_non_proxied_udp"));
        assert!(!r.command_line.contains("default"));
        assert_eq!(
            r.command_line
                .matches("--force-webrtc-ip-handling-policy")
                .count(),
            1
        );
    }

    #[test]
    fn webview2_env_insert_and_rewrite() {
        let (v, a) = ensure_webview2_arguments(None, WebRtcPolicy::ProxyOnly);
        assert_eq!(a, PolicyApply::Inserted);
        assert!(v.unwrap().contains("disable_non_proxied_udp"));

        let (v, a) = ensure_webview2_arguments(Some("--foo"), WebRtcPolicy::Strict);
        assert_eq!(a, PolicyApply::Inserted);
        let s = v.unwrap();
        assert!(s.starts_with("--foo "));
        assert!(s.contains("disable_non_proxied_udp"));

        let (v, a) = ensure_webview2_arguments(
            Some("--force-webrtc-ip-handling-policy=default"),
            WebRtcPolicy::ProxyOnly,
        );
        assert_eq!(a, PolicyApply::Rewritten);
        assert!(v.unwrap().contains("disable_non_proxied_udp"));
    }

    #[test]
    fn child_plan_unknown_unchanged() {
        let p = plan_child_policy("foo.exe", "foo.exe", WebRtcPolicy::Strict, None);
        assert_eq!(p.engine, BrowserEngine::Unknown);
        assert_eq!(p.apply, PolicyApply::Unchanged);
    }

    #[test]
    fn child_plan_webview2_uses_env() {
        let p = plan_child_policy(
            "msedgewebview2.exe",
            "msedgewebview2.exe",
            WebRtcPolicy::ProxyOnly,
            None,
        );
        assert_eq!(p.engine, BrowserEngine::WebView2);
        assert!(p
            .webview2_additional_args
            .unwrap()
            .contains("disable_non_proxied_udp"));
    }

    #[test]
    fn argv_insert_and_rewrite_no_duplicate() {
        let mut args = vec!["chrome.exe".to_string()];
        let a = ensure_chromium_webrtc_argv(&mut args, WebRtcPolicy::ProxyOnly);
        assert_eq!(a, PolicyApply::Inserted);
        assert_eq!(args.len(), 2);
        assert!(args[1].contains("disable_non_proxied_udp"));

        let a = ensure_chromium_webrtc_argv(&mut args, WebRtcPolicy::Strict);
        assert_eq!(a, PolicyApply::Rewritten);
        assert_eq!(args.len(), 2);
        assert_eq!(
            args.iter()
                .filter(|x| x.starts_with("--force-webrtc-ip-handling-policy"))
                .count(),
            1
        );
    }

    #[test]
    fn argv_host_unchanged() {
        let mut args = vec!["chrome.exe".to_string()];
        assert_eq!(
            ensure_chromium_webrtc_argv(&mut args, WebRtcPolicy::Host),
            PolicyApply::Unchanged
        );
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn audit_api_names_are_stable() {
        assert_eq!(PolicyApply::Inserted.audit_api(), "BrowserPolicyApplied");
        assert_eq!(
            PolicyApply::Rewritten.audit_api(),
            "BrowserPolicyConflictResolved"
        );
        assert_eq!(
            PolicyApply::Unchanged.audit_api(),
            "BrowserPolicyUnchanged"
        );
        assert_eq!(AUDIT_API_NETWORK_UDP_DENY, "NetworkGuardUdpDeny");
    }

    #[test]
    fn conflict_table_no_duplicate_switch() {
        // Table-driven: existing switch is always rewritten, never duplicated.
        let cases = [
            ("chrome.exe --force-webrtc-ip-handling-policy=default", WebRtcPolicy::ProxyOnly),
            ("chrome.exe --force-webrtc-ip-handling-policy=disable_non_proxied_udp --foo", WebRtcPolicy::PublicInterfaceOnly),
            ("app --force-webrtc-ip-handling-policy=default_public_interface_only", WebRtcPolicy::Strict),
        ];
        for (cmd, policy) in cases {
            let r = ensure_chromium_webrtc_switch(cmd, policy);
            assert_eq!(r.apply, PolicyApply::Rewritten, "{cmd} / {policy:?}");
            assert_eq!(
                r.command_line
                    .matches("--force-webrtc-ip-handling-policy")
                    .count(),
                1,
                "{}",
                r.command_line
            );
        }
    }

    #[test]
    fn strict_startup_fails_without_network_guard() {
        let cap = NetworkGuardCapability::browser_policy_only();
        assert!(cap.check_policy(WebRtcPolicy::Strict).is_err());
        assert_eq!(
            cap.check_policy(WebRtcPolicy::ProxyOnly).unwrap(),
            BrowserGuarantee::PolicyOnly
        );
        assert_eq!(
            cap.check_policy(WebRtcPolicy::Host).unwrap(),
            BrowserGuarantee::PolicyOnly
        );
    }

    #[test]
    fn strict_ok_when_network_guard_available() {
        let cap = NetworkGuardCapability {
            session_udp_enforced: true,
        };
        assert_eq!(
            cap.check_policy(WebRtcPolicy::Strict).unwrap(),
            BrowserGuarantee::NetworkEnforced
        );
    }

    #[test]
    fn browser_env_includes_webview2_and_fallback() {
        let e = browser_env_entries(WebRtcPolicy::Strict);
        assert!(e.iter().any(|(k, v)| k == "ENVBOX_WEBRTC_POLICY" && v == "strict"));
        assert!(e
            .iter()
            .any(|(k, _)| k == "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"));
    }
}
