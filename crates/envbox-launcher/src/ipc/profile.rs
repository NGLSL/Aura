//! Domain Profile conversion and typed DNS / identity wire fields.
use super::wire::{IpcError, IpcMessage};
use envbox_core::{DnsMode, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile};
use uuid::Uuid;

pub const RUNTIME_ENVIRONMENT_MAX: usize = 32;
pub const RUNTIME_ENVIRONMENT_ENTRY_MAX_BYTES: usize = 512;

/// Convert a domain Profile into a PROFILE message.
pub fn profile_to_message(profile: &EnvironmentProfile, instance_id: &str) -> IpcMessage {
    profile_to_message_with_flags(profile, instance_id, true, false)
}

/// PROFILE message with explicit inherit/audit flags from the Run request.
pub fn profile_to_message_with_flags(
    profile: &EnvironmentProfile,
    instance_id: &str,
    inherit_children: bool,
    audit: bool,
) -> IpcMessage {
    let mut environment: Vec<(String, String)> = profile
        .environment
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    environment.sort_by(|a, b| a.0.cmp(&b.0));
    IpcMessage::Profile {
        profile_id: profile.id.to_string(),
        instance_id: instance_id.to_string(),
        locale_name: profile.locale.locale_name.clone(),
        ui_language: profile.locale.ui_language.clone(),
        region: profile.locale.region.clone(),
        tz_windows: profile.timezone.windows_id.clone(),
        tz_iana: profile.timezone.iana_id.clone(),
        inherit_children,
        audit,
        dns_mode: matches!(profile.dns.mode, DnsMode::VirtualView),
        dns_servers: Vec::new(),
        dns_config: Some(envbox_core::DnsProfile::typed(
            profile.dns.mode.clone(),
            profile.dns.strict,
            profile.dns.effective_upstreams(),
        )),
        identity: profile.identity.clone(),
        registry_paths: profile.registry.whitelist_paths.clone(),
        environment,
        webrtc: profile.browser.webrtc.as_str().to_string(),
    }
}

/// Convert a PROFILE message back into a domain Profile.
pub fn message_to_profile(msg: &IpcMessage) -> Result<EnvironmentProfile, IpcError> {
    let IpcMessage::Profile {
        locale_name,
        ui_language,
        region,
        tz_windows,
        tz_iana,
        dns_mode,
        dns_servers,
        dns_config,
        identity,
        registry_paths,
        environment,
        webrtc,
        ..
    } = msg
    else {
        return Err(IpcError::Protocol("not a PROFILE message".into()));
    };
    if locale_name.is_empty()
        || ui_language.is_empty()
        || region.is_empty()
        || tz_windows.is_empty()
    {
        return Err(IpcError::Protocol(
            "PROFILE missing required fields (locale_name/ui_language/region/tz_windows)".into(),
        ));
    }
    let mut servers = Vec::new();
    for s in dns_servers {
        if let Ok(ip) = s.parse() {
            servers.push(ip);
        }
    }
    let webrtc_policy = if webrtc.is_empty() {
        // Older peers omit the field: Host (no browser alteration).
        envbox_core::WebRtcPolicy::Host
    } else {
        envbox_core::WebRtcPolicy::parse(webrtc)
            .ok_or_else(|| IpcError::Protocol(format!("PROFILE invalid webrtc={webrtc:?}")))?
    };
    Ok(EnvironmentProfile {
        identity: identity.clone(),
        id: Uuid::nil(),
        name: "ipc".into(),
        locale: LocaleProfile {
            locale_name: locale_name.clone(),
            ui_language: ui_language.clone(),
            region: region.clone(),
        },
        timezone: TimezoneProfile {
            windows_id: tz_windows.clone(),
            iana_id: tz_iana.clone(),
        },
        dns: dns_config.clone().unwrap_or_else(|| {
            envbox_core::DnsProfile::from_servers(
                if *dns_mode {
                    DnsMode::VirtualView
                } else {
                    DnsMode::Host
                },
                servers,
            )
        }),
        environment: environment.iter().cloned().collect(),
        registry: RegistryProfile {
            whitelist_paths: registry_paths.clone(),
        },
        browser: envbox_core::BrowserPrivacyProfile {
            webrtc: webrtc_policy,
        },
    })
}
