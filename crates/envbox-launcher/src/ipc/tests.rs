use super::*;
use envbox_core::{DnsMode, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile};
use std::collections::HashMap;
use uuid::Uuid;

#[test]
fn hello_round_trip() {
    let msg = IpcMessage::Hello {
        pid: 42,
        instance_id: "abc-def".into(),
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn get_profile_round_trip() {
    let msg = IpcMessage::GetProfile {
        pid: 7,
        profile_id: "p1".into(),
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn all_public_message_names_encode() {
    let msgs = [
        IpcMessage::Hello {
            pid: 1,
            instance_id: "x".into(),
        },
        IpcMessage::GetProfile {
            pid: 1,
            profile_id: "x".into(),
        },
        IpcMessage::Profile {
            profile_id: "x".into(),
            instance_id: "i".into(),
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
            tz_windows: "Pacific Standard Time".into(),
            tz_iana: "America/Los_Angeles".into(),
            inherit_children: true,
            audit: false,
            dns_mode: true,
            dns_servers: vec!["1.1.1.1".into()],
            dns_config: None,
            identity: Default::default(),
            registry_paths: vec!["HKCU\\Software\\EnvBox".into()],
            environment: vec![("LANG".into(), "en_US.UTF-8".into())],
            webrtc: "proxy_only".into(),
        },
        IpcMessage::RuntimeReady { pid: 1 },
        IpcMessage::HookError {
            pid: 1,
            api: "GetTimeZoneInformation".into(),
            code: 5,
            detail: "access denied".into(),
        },
        IpcMessage::ProcessCreated {
            pid: 1,
            child_pid: 2,
            image: r"C:\a b\app.exe".into(),
        },
        IpcMessage::ProcessExited {
            pid: 2,
            exit_code: 0,
        },
    ];
    for m in msgs {
        assert_eq!(round_trip(&m), m, "round-trip failed for {}", m.name());
    }
}

#[test]
fn quoted_values_with_spaces_and_escapes() {
    let msg = IpcMessage::HookError {
        pid: 1,
        api: "GetTimeZoneInformation".into(),
        code: 5,
        detail: "a b=c \"x\"".into(),
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn profile_message_round_trip() {
    let profile = EnvironmentProfile {
        identity: Default::default(),
        id: Uuid::nil(),
        name: "US".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: envbox_core::DnsProfile {
            mode: DnsMode::VirtualView,
            servers: vec!["1.1.1.1".parse().unwrap()],
            ..Default::default()
        },
        environment: HashMap::from([
            ("LANG".into(), "en_US.UTF-8".into()),
            ("TOOL_OPTIONS".into(), "alpha beta=gamma".into()),
        ]),
        registry: RegistryProfile {
            whitelist_paths: vec!["HKCU\\Software\\EnvBox".into()],
        },
        browser: Default::default(),
    };
    let msg = profile_to_message(&profile, "inst-1");
    let back = round_trip(&msg);
    let decoded = message_to_profile(&back).unwrap();
    assert_eq!(decoded.locale, profile.locale);
    assert_eq!(decoded.timezone, profile.timezone);
    assert_eq!(decoded.dns.servers, profile.dns.servers);
    assert_eq!(decoded.environment, profile.environment);
    assert_eq!(
        decoded.registry.whitelist_paths,
        profile.registry.whitelist_paths
    );
    assert_eq!(decoded.browser.webrtc, profile.browser.webrtc);
}

#[test]
fn runtime_without_with_token_boundary_cannot_be_accepted() {
    let mut observed = ObservedRuntimeIdentity {
        identity: RuntimeIdentity {
            pid: 1,
            creation_time: 1,
            protocol: RUNTIME_IDENTITY_PROTOCOL,
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            module_path: "fixture-runtime.dll".into(),
            actual_profile: "PROFILE profile_id=p instance_id=i locale_name=en-US ui_language=en-US region=US tz_windows=UTC tz_iana=Etc/UTC inherit_children=1 audit=0 webrtc=host dns_mode=0".into(),
            config_complete: true,
            hooks: [
                ("time", 8), ("geo", 2), ("locale", 14), ("language", 6),
                ("registry", 7), ("dns", 2), ("process", 3),
                ("network_policy", 1),
            ].into_iter().map(|(name, count)| (name.into(), count)).collect(),
        },
        module_sha256: "fixture".into(),
        config_sha256: "fixture".into(),
    };
    let rejected = SessionTable::validate_observation(&observed).unwrap_err();
    assert!(rejected
        .to_string()
        .contains("required hook set incomplete: process"));
    observed
        .identity
        .hooks
        .iter_mut()
        .find(|(name, _)| name == "process")
        .unwrap()
        .1 = 4;
    assert!(SessionTable::validate_observation(&observed).is_ok());
    // Duplicating a count does not establish another attached API.
    observed.identity.hooks.push(("process".into(), 4));
    assert!(SessionTable::validate_observation(&observed).is_err());
    observed.identity.hooks.pop();
    observed
        .identity
        .actual_profile
        .push_str(" identity_user_name=tester");
    assert!(SessionTable::validate_observation(&observed).is_err());
    observed.identity.hooks.push(("identity".into(), 2));
    assert!(SessionTable::validate_observation(&observed).is_ok());
    observed
        .identity
        .actual_profile
        .push_str(" identity_computer_name=aura-test identity_mac_address=02:11:22:33:44:55");
    assert!(SessionTable::validate_observation(&observed).is_err());
    observed.identity.hooks.last_mut().unwrap().1 = 13;
    assert!(SessionTable::validate_observation(&observed).is_ok());
    observed.identity.hooks.push(("identity".into(), 13));
    assert!(SessionTable::validate_observation(&observed).is_err());
}

#[test]
fn doh_ipc_preserves_policy_and_rejects_missing_or_unknown_values() {
    let mut profile = EnvironmentProfile {
        identity: Default::default(),
        id: Uuid::nil(),
        name: "DoH".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: Default::default(),
        environment: Default::default(),
        registry: Default::default(),
        browser: Default::default(),
    };
    profile.dns = envbox_core::DnsProfile::typed(
        envbox_core::DnsMode::VirtualView,
        true,
        vec![envbox_core::DnsUpstream::Doh {
            url: "https://1.1.1.1/dns-query".into(),
            bootstrap_ips: vec![],
            tls_revocation: envbox_core::DnsTlsRevocation::StrictOffline,
        }],
    );
    let line = profile_to_message(&profile, "instance").encode_line();
    assert!(line.contains("dns_upstream_0_tls_revocation=1"));
    let decoded = IpcMessage::decode_line(&line).unwrap();
    assert_eq!(message_to_profile(&decoded).unwrap().dns, profile.dns);
    assert!(
        IpcMessage::decode_line(&line.replace(" dns_upstream_0_tls_revocation=1", "")).is_err()
    );
    assert!(IpcMessage::decode_line(&line.replace(
        "dns_upstream_0_tls_revocation=1",
        "dns_upstream_0_tls_revocation=2"
    ))
    .is_err());
}

#[test]
fn profile_webrtc_round_trips_and_invalid_rejected() {
    let mut profile = EnvironmentProfile {
        identity: Default::default(),
        id: Uuid::nil(),
        name: "US".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: envbox_core::DnsProfile {
            mode: DnsMode::Host,
            servers: vec![],
            ..Default::default()
        },
        environment: HashMap::new(),
        registry: RegistryProfile::default(),
        browser: Default::default(),
    };
    profile.browser.webrtc = envbox_core::WebRtcPolicy::Strict;
    let msg = profile_to_message(&profile, "inst-1");
    let decoded = message_to_profile(&msg).unwrap();
    assert_eq!(decoded.browser.webrtc, envbox_core::WebRtcPolicy::Strict);

    // Empty webrtc (older peer) → Host.
    if let IpcMessage::Profile { webrtc, .. } = &msg {
        let mut m = msg.clone();
        if let IpcMessage::Profile { webrtc: w, .. } = &mut m {
            *w = String::new();
        }
        let _ = webrtc;
        let decoded = message_to_profile(&m).unwrap();
        assert_eq!(decoded.browser.webrtc, envbox_core::WebRtcPolicy::Host);

        // Garbage webrtc → protocol error (never silent Host).
        if let IpcMessage::Profile { webrtc: w, .. } = &mut m {
            *w = "nope".into();
        }
        assert!(message_to_profile(&m).is_err());
    }
}

#[test]
fn fake_broker_serves_profile() {
    let profile = EnvironmentProfile {
        identity: Default::default(),
        id: Uuid::nil(),
        name: "US".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: envbox_core::DnsProfile {
            mode: DnsMode::Host,
            servers: vec![],
            ..Default::default()
        },
        environment: HashMap::new(),
        registry: RegistryProfile::default(),
        browser: Default::default(),
    };
    let mut broker = FakeBroker::new();
    broker.table.set_instance_id("inst");
    // Use a fixed id so bind_pid can resolve it.
    let mut p = profile;
    p.id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    broker.table.register_profile(&p);
    broker.table.bind_pid(10, &p.id.to_string());
    let reply = broker
        .send(IpcMessage::GetProfile {
            pid: 10,
            profile_id: String::new(),
        })
        .expect("profile reply");
    assert_eq!(reply.name(), "PROFILE");
}

#[test]
fn profile_requires_core_fields() {
    let msg = IpcMessage::Profile {
        profile_id: "x".into(),
        instance_id: "i".into(),
        locale_name: String::new(),
        ui_language: "en-US".into(),
        region: "US".into(),
        tz_windows: "PST".into(),
        tz_iana: "UTC".into(),
        inherit_children: true,
        audit: false,
        dns_mode: false,
        dns_servers: vec![],
        dns_config: None,
        identity: Default::default(),
        registry_paths: vec![],
        environment: vec![],
        webrtc: "host".into(),
    };
    assert!(message_to_profile(&msg).is_err());
}

#[test]
fn register_and_bind_round_trip() {
    let reg = IpcMessage::RegisterProfile {
        profile_id: "p1".into(),
        instance_id: "i".into(),
        locale_name: "en-US".into(),
        ui_language: "en-US".into(),
        region: "US".into(),
        tz_windows: "Pacific Standard Time".into(),
        tz_iana: "America/Los_Angeles".into(),
        inherit_children: true,
        audit: false,
        dns_mode: false,
        dns_servers: vec![],
        dns_config: None,
        identity: Default::default(),
        registry_paths: vec![],
        environment: vec![("LANG".into(), "en_US.UTF-8".into())],
        webrtc: "strict".into(),
    };
    assert_eq!(round_trip(&reg), reg);
    let bind = IpcMessage::BindPid {
        pid: 10,
        profile_id: "p1".into(),
        parent_pid: 1,
    };
    assert_eq!(round_trip(&bind), bind);
}

#[test]
fn session_registry_tracks_children_and_exit() {
    let mut t = SessionTable::new();
    t.handle(&IpcMessage::RegisterProfile {
        profile_id: "p1".into(),
        instance_id: "i".into(),
        locale_name: "en-US".into(),
        ui_language: "en-US".into(),
        region: "US".into(),
        tz_windows: "PST".into(),
        tz_iana: "UTC".into(),
        inherit_children: true,
        audit: false,
        dns_mode: false,
        dns_servers: vec![],
        dns_config: None,
        identity: Default::default(),
        registry_paths: vec![],
        environment: vec![("LANG".into(), "en_US.UTF-8".into())],
        webrtc: "host".into(),
    });
    t.handle(&IpcMessage::BindPid {
        pid: 10,
        profile_id: "p1".into(),
        parent_pid: 0,
    });
    t.handle(&IpcMessage::ProcessCreated {
        pid: 10,
        child_pid: 11,
        image: "child.exe".into(),
    });
    assert_eq!(t.profile_of(11), Some("p1"));
    assert_eq!(t.live_pids("p1"), vec![10, 11]);
    t.handle(&IpcMessage::ProcessExited {
        pid: 11,
        exit_code: 0,
    });
    assert_eq!(t.live_pids("p1"), vec![10]);
}
#[test]
fn identity_ipc_is_optional_strict_and_ordered() {
    let base = "PROFILE profile_id=p instance_id=i locale_name=en-US ui_language=en-US region=US tz_windows=UTC tz_iana=UTC inherit_children=1 audit=0 dns_mode=0 webrtc=host";
    let old = IpcMessage::decode_line(base).unwrap();
    assert!(!old.encode_line().contains("identity_"));
    let line = format!("{base} identity_computer_name=aura-test identity_user_name=tester identity_mac_address=02:11:22:33:44:55 identity_machine_guid=12345678-1234-1234-1234-123456789abc");
    let msg = IpcMessage::decode_line(&line).unwrap();
    let profile = message_to_profile(&msg).unwrap();
    assert_eq!(profile.identity.computer_name.as_deref(), Some("aura-test"));
    assert_eq!(IpcMessage::decode_line(&msg.encode_line()).unwrap(), msg);
    for suffix in [
        "identity_unknown=x",
        "identity_user_name=",
        "identity_user_name=a identity_user_name=b",
        "identity_mac_address=FF:FF:FF:FF:FF:FF",
        "identity_machine_guid=00000000-0000-0000-0000-000000000000",
    ] {
        assert!(
            IpcMessage::decode_line(&format!("{base} {suffix}")).is_err(),
            "{suffix}"
        );
    }
}
