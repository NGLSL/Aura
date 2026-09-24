//! Environment Block construction (Unicode CreateProcess form).
//! Windows env names are case-insensitive: Profile overrides replace case-insensitively.

use envbox_core::EnvironmentProfile;
use std::collections::HashMap;
use uuid::Uuid;

/// Clone host environment, apply Profile overrides (case-insensitive keys), then EnvBox IDs.
/// `inherit_children` controls child process Profile propagation (Application flag).
pub fn build_environment_block(
    host: &HashMap<String, String>,
    profile: Option<&EnvironmentProfile>,
    instance_id: Uuid,
    profile_id: Uuid,
    inherit_children: bool,
) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = host.clone();
    if let Some(profile) = profile {
        for (key, value) in &profile.environment {
            // Drop any host key that matches case-insensitively before insert.
            let lower = key.to_ascii_lowercase();
            env.retain(|k, _| k.to_ascii_lowercase() != lower);
            env.insert(key.clone(), value.clone());
        }
    }
    let lower = |k: &str| k.to_ascii_lowercase();
    env.retain(|k, _| {
        lower(k) != "envbox_instance_id"
            && lower(k) != "envbox_profile_id"
            && lower(k) != "envbox_inherit_children"
    });
    env.insert("ENVBOX_INSTANCE_ID".into(), instance_id.to_string());
    env.insert("ENVBOX_PROFILE_ID".into(), profile_id.to_string());
    env.insert(
        "ENVBOX_INHERIT_CHILDREN".into(),
        if inherit_children { "1" } else { "0" }.into(),
    );
    env
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
            },
            environment: HashMap::from([("LANG".into(), "en_US.UTF-8".into())]),
            registry: RegistryProfile::default(),
        }
    }

    #[test]
    fn environment_block_applies_profile_and_internal_ids() {
        let host = HashMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("PATH".into(), r"C:\Windows".into()),
        ]);
        let merged = build_environment_block(&host, Some(&profile()), Uuid::nil(), Uuid::nil(), true);
        assert_eq!(merged.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
        assert_eq!(merged.get("PATH").map(String::as_str), Some(r"C:\Windows"));
        assert!(merged.contains_key("ENVBOX_INSTANCE_ID"));
        assert!(merged.contains_key("ENVBOX_PROFILE_ID"));
        assert_eq!(
            merged.get("ENVBOX_INHERIT_CHILDREN").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn inherit_children_flag_written() {
        let host = HashMap::new();
        let merged = build_environment_block(&host, None, Uuid::nil(), Uuid::nil(), false);
        assert_eq!(
            merged.get("ENVBOX_INHERIT_CHILDREN").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn profile_override_is_case_insensitive() {
        let host = HashMap::from([("lang".into(), "zh_CN.UTF-8".into())]);
        let merged = build_environment_block(&host, Some(&profile()), Uuid::nil(), Uuid::nil(), true);
        // Exactly one LANG-ish key, value from profile.
        let langs: Vec<_> = merged
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("lang"))
            .collect();
        assert_eq!(langs.len(), 1);
        assert_eq!(langs[0].1, "en_US.UTF-8");
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
}
