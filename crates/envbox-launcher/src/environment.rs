//! Environment Block construction (Unicode CreateProcess form).
//! Windows env names are case-insensitive: Profile overrides replace case-insensitively.

use envbox_core::EnvironmentProfile;
use std::collections::HashMap;
use uuid::Uuid;

/// Clone host environment, apply Profile overrides (case-insensitive keys), then EnvBox IDs.
/// `inherit_children` controls child process Profile propagation (Application flag).
/// `audit` enables Audit Mode (ticket 20) via `ENVBOX_AUDIT`.
pub fn build_environment_block(
    host: &HashMap<String, String>,
    profile: Option<&EnvironmentProfile>,
    instance_id: Uuid,
    profile_id: Uuid,
    inherit_children: bool,
    audit: bool,
) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = host.clone();
    if let Some(profile) = profile {
        // A Profile locale must not inherit conflicting POSIX locale values
        // from the host. Explicit Profile LANG / LC_* / LANGUAGE values are
        // reinserted below, retaining their intended precedence.
        env.retain(|key, _| {
            !key.eq_ignore_ascii_case("LANG")
                && !key.eq_ignore_ascii_case("LANGUAGE")
                && !key.to_ascii_uppercase().starts_with("LC_")
        });
        for (key, value) in &profile.environment {
            // Drop any host key that matches case-insensitively before insert.
            let lower = key.to_ascii_lowercase();
            env.retain(|k, _| k.to_ascii_lowercase() != lower);
            env.insert(key.clone(), value.clone());
        }
    }
    let lower = |k: &str| k.to_ascii_lowercase();
    const DROP: [&str; 16] = [
        "envbox_recovery_job_name",
        "envbox_startup_gate",
        "envbox_ipc_pipe",
        "envbox_instance_id",
        "envbox_profile_id",
        "envbox_inherit_children",
        "envbox_audit",
        "envbox_locale_name",
        "envbox_ui_language",
        "envbox_region",
        "envbox_tz_windows",
        "envbox_tz_iana",
        "envbox_dns_mode",
        "envbox_dns_servers",
        "envbox_registry_paths",
        "envbox_webrtc_policy",
    ];
    env.retain(|k, _| {
        !DROP.contains(&lower(k).as_str())
            && !lower(k).starts_with("envbox_dns_upstream_")
            && !matches!(
                lower(k).as_str(),
                "envbox_dns_config_version" | "envbox_dns_strict" | "envbox_dns_config_error"
            )
    });
    // Host WebRTC policy must not strip user WEBVIEW2 args (spec: Host 不覆盖).
    // Non-Host policies rewrite WEBVIEW2 below via browser_env_entries.
    if profile
        .map(|p| p.browser.webrtc != envbox_core::WebRtcPolicy::Host)
        .unwrap_or(false)
    {
        env.retain(|k, _| lower(k) != "webview2_additional_browser_arguments");
    }
    env.insert("ENVBOX_INSTANCE_ID".into(), instance_id.to_string());
    env.insert("ENVBOX_PROFILE_ID".into(), profile_id.to_string());
    env.insert(
        "ENVBOX_INHERIT_CHILDREN".into(),
        if inherit_children { "1" } else { "0" }.into(),
    );
    env.insert("ENVBOX_AUDIT".into(), if audit { "1" } else { "0" }.into());
    // ENVBOX_* value fallback (no C++ TOML). Used when Broker IPC is down.
    if let Some(profile) = profile {
        insert_profile_value_fallback(&mut env, profile);
    }
    env
}

/// Write structured Profile fields as ENVBOX_* (Win32 fallback channel).
/// Runtime prefers Broker PROFILE; these vars are the non-TOML fallback.
pub fn insert_profile_value_fallback(
    env: &mut HashMap<String, String>,
    profile: &EnvironmentProfile,
) {
    env.retain(|key, _| {
        let key = key.to_ascii_lowercase();
        !key.starts_with("envbox_dns_upstream_")
            && !matches!(
                key.as_str(),
                "envbox_dns_config_version" | "envbox_dns_strict" | "envbox_dns_config_error"
            )
    });
    // Startup validates the complete payload first. Invalid sentinels also prevent a
    // typed decoder from interpreting encoding failure as an empty Host configuration.
    match checked_dns_environment(&profile.dns) {
        Ok(values) => {
            for (key, value) in values {
                env.insert(key, value);
            }
        }
        Err(err) => {
            env.insert("ENVBOX_DNS_CONFIG_VERSION".into(), "invalid".into());
            env.insert("ENVBOX_DNS_CONFIG_ERROR".into(), err.to_string());
            env.insert("ENVBOX_DNS_STRICT".into(), "1".into());
        }
    }
    env.insert(
        "ENVBOX_LOCALE_NAME".into(),
        profile.locale.locale_name.clone(),
    );
    env.insert(
        "ENVBOX_UI_LANGUAGE".into(),
        profile.locale.ui_language.clone(),
    );
    env.insert("ENVBOX_REGION".into(), profile.locale.region.clone());
    env.insert(
        "ENVBOX_TZ_WINDOWS".into(),
        profile.timezone.windows_id.clone(),
    );
    env.insert("ENVBOX_TZ_IANA".into(), profile.timezone.iana_id.clone());
    env.insert(
        "ENVBOX_DNS_MODE".into(),
        match profile.dns.mode {
            envbox_core::DnsMode::Host => "0".into(),
            envbox_core::DnsMode::VirtualView => "1".into(),
        },
    );
    env.insert(
        "ENVBOX_DNS_SERVERS".into(),
        profile
            .dns
            .servers
            .iter()
            .map(|ip| ip.to_string())
            .collect::<Vec<_>>()
            .join(";"),
    );
    env.insert(
        "ENVBOX_REGISTRY_PATHS".into(),
        profile.registry.whitelist_paths.join(";"),
    );
    // Browser / Network Guard (WebRTC Privacy): policy token + WebView2 args.
    for (key, value) in envbox_core::browser_env_entries(profile.browser.webrtc) {
        // Replace case-insensitively so Host WebView2 args cannot win over Profile.
        let lower = key.to_ascii_lowercase();
        env.retain(|k, _| k.to_ascii_lowercase() != lower);
        env.insert(key, value);
    }
}

/// Checked typed DNS fallback fields, using the same v1 vocabulary as IPC.
pub fn checked_dns_environment(
    dns: &envbox_core::DnsProfile,
) -> Result<HashMap<String, String>, envbox_core::DomainError> {
    Ok(dns
        .flat_fields()?
        .into_iter()
        .map(|(key, value)| (format!("ENVBOX_{}", key.to_ascii_uppercase()), value))
        .collect())
}

/// Encode as a Windows Unicode environment block: `k=v\0k=v\0\0`.
pub fn encode_environment_block(env: &HashMap<String, String>) -> Vec<u16> {
    let mut pairs: Vec<(&String, &String)> = env.iter().collect();
    pairs.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));
    let mut out = Vec::new();
    for (key, value) in pairs {
        out.extend(key.encode_utf16());
        out.push(u16::from(b'='));
        out.extend(value.encode_utf16());
        out.push(0);
    }
    out.push(0);
    if out.len() == 1 {
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use envbox_core::{DnsMode, DnsProfile, LocaleProfile, RegistryProfile, TimezoneProfile};

    fn profile() -> EnvironmentProfile {
        EnvironmentProfile {
            id: Uuid::nil(),
            name: "US Development".into(),
            locale: LocaleProfile {
                locale_name: "en-US".into(),
                ui_language: "en-US".into(),
                region: "US".into(),
            },
            timezone: TimezoneProfile {
                windows_id: "Pacific Standard Time".into(),
                iana_id: "America/Los_Angeles".into(),
            },
            dns: DnsProfile {
                mode: DnsMode::Host,
                servers: vec![],
                ..Default::default()
            },
            environment: HashMap::from([("LANG".into(), "en_US.UTF-8".into())]),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        }
    }

    #[test]
    fn environment_block_applies_profile_and_internal_ids() {
        let host = HashMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("PATH".into(), r"C:\Windows".into()),
        ]);
        let merged = build_environment_block(
            &host,
            Some(&profile()),
            Uuid::nil(),
            Uuid::nil(),
            true,
            false,
        );
        assert_eq!(merged.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
        assert_eq!(merged.get("PATH").map(String::as_str), Some(r"C:\Windows"));
        assert!(merged.contains_key("ENVBOX_INSTANCE_ID"));
        assert!(merged.contains_key("ENVBOX_PROFILE_ID"));
        assert_eq!(
            merged.get("ENVBOX_INHERIT_CHILDREN").map(String::as_str),
            Some("1")
        );
        assert_eq!(merged.get("ENVBOX_AUDIT").map(String::as_str), Some("0"));
    }

    #[test]
    fn inherit_children_flag_written() {
        let host = HashMap::new();
        let merged = build_environment_block(&host, None, Uuid::nil(), Uuid::nil(), false, true);
        assert_eq!(
            merged.get("ENVBOX_INHERIT_CHILDREN").map(String::as_str),
            Some("0")
        );
        assert_eq!(merged.get("ENVBOX_AUDIT").map(String::as_str), Some("1"));
    }

    #[test]
    fn profile_value_fallback_written_without_toml() {
        let host = HashMap::new();
        let mut p = profile();
        p.dns = envbox_core::DnsProfile {
            mode: envbox_core::DnsMode::VirtualView,
            servers: vec!["1.1.1.1".parse().unwrap(), "8.8.8.8".parse().unwrap()],
            ..Default::default()
        };
        p.registry = RegistryProfile {
            whitelist_paths: vec!["HKCU\\Software\\EnvBox".into()],
        };
        let merged =
            build_environment_block(&host, Some(&p), Uuid::nil(), Uuid::nil(), true, false);
        assert_eq!(
            merged.get("ENVBOX_LOCALE_NAME").map(String::as_str),
            Some("en-US")
        );
        assert_eq!(
            merged.get("ENVBOX_TZ_WINDOWS").map(String::as_str),
            Some("Pacific Standard Time")
        );
        assert_eq!(merged.get("ENVBOX_DNS_MODE").map(String::as_str), Some("1"));
        assert_eq!(
            merged.get("ENVBOX_DNS_SERVERS").map(String::as_str),
            Some("1.1.1.1;8.8.8.8")
        );
        assert_eq!(
            merged.get("ENVBOX_REGISTRY_PATHS").map(String::as_str),
            Some("HKCU\\Software\\EnvBox")
        );
    }

    #[test]
    fn profile_override_is_case_insensitive() {
        let host = HashMap::from([("lang".into(), "zh_CN.UTF-8".into())]);
        let merged = build_environment_block(
            &host,
            Some(&profile()),
            Uuid::nil(),
            Uuid::nil(),
            true,
            false,
        );
        // Exactly one LANG-ish key, value from profile.
        let langs: Vec<_> = merged
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("lang"))
            .collect();
        assert_eq!(langs.len(), 1);
        assert_eq!(langs[0].1, "en_US.UTF-8");
    }

    #[test]
    fn profile_lang_does_not_leave_inherited_locale_overrides() {
        let host = HashMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("LC_MESSAGES".into(), "zh_CN.UTF-8".into()),
            ("LANGUAGE".into(), "zh_CN".into()),
            ("PATH".into(), r"C:\Windows".into()),
        ]);
        let mut p = profile();
        p.environment.insert("LC_TIME".into(), "en_US.UTF-8".into());
        let merged =
            build_environment_block(&host, Some(&p), Uuid::nil(), Uuid::nil(), true, false);
        assert_eq!(merged.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
        assert_eq!(
            merged.get("LC_TIME").map(String::as_str),
            Some("en_US.UTF-8")
        );
        for key in ["LC_ALL", "LC_MESSAGES", "LANGUAGE"] {
            assert!(!merged.contains_key(key), "inherited {key} overrides LANG");
        }
        assert!(merged.contains_key("PATH"));

        let plain = build_environment_block(&host, None, Uuid::nil(), Uuid::nil(), true, false);
        assert_eq!(plain.get("LC_ALL").map(String::as_str), Some("C.UTF-8"));
    }

    #[test]
    fn profile_locale_without_lang_does_not_inherit_host_locale_values() {
        let host = HashMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            ("LANGUAGE".into(), "zh_CN".into()),
            ("PATH".into(), r"C:\Windows".into()),
        ]);
        let mut p = profile();
        p.environment.clear();
        p.environment.insert("LC_TIME".into(), "en_US.UTF-8".into());
        let merged =
            build_environment_block(&host, Some(&p), Uuid::nil(), Uuid::nil(), true, false);
        for key in ["LANG", "LC_ALL", "LANGUAGE"] {
            assert!(!merged.contains_key(key), "inherited host {key}");
        }
        assert_eq!(
            merged.get("LC_TIME").map(String::as_str),
            Some("en_US.UTF-8")
        );
        assert!(merged.contains_key("PATH"));
    }

    #[test]
    fn browser_policy_written_to_env_block() {
        let host = HashMap::new();
        let mut p = profile();
        p.browser.webrtc = envbox_core::WebRtcPolicy::ProxyOnly;
        let merged =
            build_environment_block(&host, Some(&p), Uuid::nil(), Uuid::nil(), true, false);
        assert_eq!(
            merged.get("ENVBOX_WEBRTC_POLICY").map(String::as_str),
            Some("proxy_only")
        );
        let webview = merged
            .get("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
            .map(String::as_str)
            .unwrap_or_default();
        assert!(webview.contains("disable_non_proxied_udp"));
    }

    #[test]
    fn browser_host_policy_omits_webview2_args() {
        let host = HashMap::new();
        let merged = build_environment_block(
            &host,
            Some(&profile()),
            Uuid::nil(),
            Uuid::nil(),
            true,
            false,
        );
        assert_eq!(
            merged.get("ENVBOX_WEBRTC_POLICY").map(String::as_str),
            Some("host")
        );
        assert!(!merged.contains_key("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"));
    }

    #[test]
    fn browser_host_policy_preserves_user_webview2_args() {
        let host = HashMap::from([(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS".into(),
            "--user-flag".into(),
        )]);
        let merged = build_environment_block(
            &host,
            Some(&profile()),
            Uuid::nil(),
            Uuid::nil(),
            true,
            false,
        );
        assert_eq!(
            merged
                .get("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
                .map(String::as_str),
            Some("--user-flag")
        );
    }

    #[test]
    fn browser_non_host_rewrites_webview2_args() {
        let host = HashMap::from([(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS".into(),
            "--force-webrtc-ip-handling-policy=default --keep".into(),
        )]);
        let mut p = profile();
        p.browser.webrtc = envbox_core::WebRtcPolicy::ProxyOnly;
        let merged =
            build_environment_block(&host, Some(&p), Uuid::nil(), Uuid::nil(), true, false);
        let v = merged
            .get("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS")
            .map(String::as_str)
            .unwrap_or_default();
        assert!(v.contains("disable_non_proxied_udp"));
        assert!(!v.contains("policy=default"));
    }

    #[test]
    fn encode_uses_unicode_env_block_shape() {
        let mut env = HashMap::new();
        env.insert("A".to_string(), "1".to_string());
        env.insert("B".to_string(), "2".to_string());
        let block = encode_environment_block(&env);
        assert_eq!(block.last(), Some(&0));
        let text: String = block
            .iter()
            .map(|&u| char::from_u32(u as u32).unwrap_or('\0'))
            .collect();
        assert!(text.contains("A=1\0"));
        assert!(text.contains("B=2\0"));
    }

    #[test]
    fn typed_dns_preserves_ports_and_removes_inherited_upstreams() {
        let host = HashMap::from([
            ("envbox_dns_upstream_7_address".into(), "8.8.8.8".into()),
            ("ENVBOX_DNS_CONFIG_ERROR".into(), "stale".into()),
        ]);
        let mut p = profile();
        p.dns = DnsProfile::typed(
            DnsMode::VirtualView,
            true,
            vec![
                envbox_core::DnsUpstream::Tcp {
                    address: "127.0.0.1".parse().unwrap(),
                    port: 15353,
                },
                envbox_core::DnsUpstream::Udp {
                    address: "127.0.0.1".parse().unwrap(),
                    port: 15354,
                },
            ],
        );
        let merged =
            build_environment_block(&host, Some(&p), Uuid::nil(), Uuid::nil(), true, false);
        assert_eq!(
            merged.get("ENVBOX_DNS_CONFIG_VERSION").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            merged.get("ENVBOX_DNS_UPSTREAM_0_TYPE").map(String::as_str),
            Some("tcp")
        );
        assert_eq!(
            merged.get("ENVBOX_DNS_UPSTREAM_0_PORT").map(String::as_str),
            Some("15353")
        );
        assert_eq!(
            merged.get("ENVBOX_DNS_UPSTREAM_1_PORT").map(String::as_str),
            Some("15354")
        );
        assert!(!merged.contains_key("ENVBOX_DNS_CONFIG_ERROR"));
        assert!(!merged
            .keys()
            .any(|key| key.eq_ignore_ascii_case("ENVBOX_DNS_UPSTREAM_7_ADDRESS")));
    }

    #[test]
    fn invalid_dns_environment_uses_failure_sentinel() {
        let mut p = profile();
        p.dns = DnsProfile::typed(
            DnsMode::VirtualView,
            true,
            vec![envbox_core::DnsUpstream::Udp {
                address: "127.0.0.1".parse().unwrap(),
                port: 0,
            }],
        );
        assert!(checked_dns_environment(&p.dns).is_err());
        let merged = build_environment_block(
            &HashMap::new(),
            Some(&p),
            Uuid::nil(),
            Uuid::nil(),
            true,
            false,
        );
        assert_eq!(
            merged.get("ENVBOX_DNS_CONFIG_VERSION").map(String::as_str),
            Some("invalid")
        );
        assert_eq!(
            merged.get("ENVBOX_DNS_STRICT").map(String::as_str),
            Some("1")
        );
        assert!(merged.contains_key("ENVBOX_DNS_CONFIG_ERROR"));
        assert!(!merged.contains_key("ENVBOX_DNS_UPSTREAM_0_ADDRESS"));
    }
}
