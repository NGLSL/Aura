use envbox_core::{DnsProfile, DnsUpstream};

#[test]
fn legacy_servers_migrate_order_and_strict_without_host_fallback() {
    let dns: DnsProfile =
        toml::from_str("mode = 'virtual_view'\nservers = ['1.1.1.1', '1.0.0.1']\n").unwrap();
    assert!(dns.strict);
    assert_eq!(
        dns.upstreams,
        vec![
            DnsUpstream::Udp {
                address: "1.1.1.1".parse().unwrap(),
                port: 53
            },
            DnsUpstream::Udp {
                address: "1.0.0.1".parse().unwrap(),
                port: 53
            }
        ]
    );
    dns.validate_runtime_support().unwrap();
    let wire = toml::to_string(&dns).unwrap();
    assert!(wire.contains("upstreams") && !wire.contains("servers"));
    assert_eq!(toml::from_str::<DnsProfile>(&wire).unwrap(), dns);
}

#[test]
fn encrypted_configuration_roundtrips_and_is_runtime_supported() {
    let wire = "mode = 'virtual_view'\nstrict = true\n[[upstreams]]\ntype = 'doh'\nurl = 'https://dns.example/dns-query'\nbootstrap_ips = ['1.1.1.1']\n[[upstreams]]\ntype = 'dot'\naddress = '1.1.1.1'\nport = 853\nserver_name = 'dns.example'\n[[upstreams]]\ntype = 'tcp'\naddress = '1.0.0.1'\nport = 53\n[[upstreams]]\ntype = 'udp'\naddress = '1.0.0.1'\nport = 53\n";
    let dns: DnsProfile = toml::from_str(wire).unwrap();
    dns.validate().unwrap();
    assert_eq!(dns.upstreams.len(), 4);
    assert!(dns.servers.is_empty());
    dns.validate_runtime_support().unwrap();
    assert_eq!(
        toml::from_str::<DnsProfile>(&toml::to_string(&dns).unwrap()).unwrap(),
        dns
    );
}

#[test]
fn malformed_mixed_and_unimplemented_dns_are_rejected_explicitly() {
    for wire in [
        "mode='virtual_view'\nservers=['1.1.1.1']\nupstreams=[]\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='udp'\naddress='1.1.1.1'\nport=0\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='tcp'\naddress='resolver.example'\nport=53\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='dot'\naddress='1.1.1.1'\nserver_name=''\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='doh'\nurl='https://dns.example/dns-query'\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='doh'\nurl='http://1.1.1.1/dns-query'\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='doh'\nurl='https://user:secret@dns.example/dns-query'\nbootstrap_ips=['1.1.1.1']\n",
        "mode='virtual_view'\n[[upstreams]]\ntype='doh'\nurl='https://dns.example/dns-query'\nbootstrap_ips=['resolver.example']\n",
    ] { assert!(toml::from_str::<DnsProfile>(wire).is_err(), "{wire}"); }
    let non_strict: DnsProfile =
        toml::from_str("mode='virtual_view'\nstrict=false\nservers=['1.1.1.1']\n").unwrap();
    assert!(!non_strict.strict);
    assert!(non_strict.validate_runtime_support().is_err());
    let ip_url: DnsProfile = toml::from_str(
        "mode='virtual_view'\n[[upstreams]]\ntype='doh'\nurl='https://[::1]:8443/dns-query'\n",
    )
    .unwrap();
    assert!(ip_url.servers.is_empty());
    ip_url.validate_runtime_support().unwrap();
}

#[test]
fn flat_fields_preserve_order_and_reject_payload_overflow() {
    let mut dns = envbox_core::DnsProfile::typed(
        envbox_core::DnsMode::VirtualView,
        true,
        vec![
            DnsUpstream::Dot {
                address: "1.1.1.1".parse().unwrap(),
                port: 853,
                server_name: "dns.example".into(),
            },
            DnsUpstream::Udp {
                address: "1.0.0.1".parse().unwrap(),
                port: 53,
            },
        ],
    );
    let fields = dns.flat_fields().unwrap();
    assert!(fields.contains(&("dns_upstream_0_type".into(), "dot".into())));
    assert!(fields.contains(&("dns_upstream_1_type".into(), "udp".into())));
    dns.validate_runtime_support().unwrap();
    dns = envbox_core::DnsProfile::typed(
        envbox_core::DnsMode::VirtualView,
        true,
        vec![
            DnsUpstream::Doh {
                url: format!("https://dns.example/{}", "x".repeat(1900)),
                bootstrap_ips: vec!["1.1.1.1".parse().unwrap()],
                tls_revocation: envbox_core::DnsTlsRevocation::Standard
            };
            8
        ],
    );
    assert!(dns.flat_fields().is_err());
    assert!(dns.validate_runtime_support().is_err());
}

#[test]
fn native_udp_tcp_port_capability_is_separate_from_encrypted_transports() {
    let dns = DnsProfile::typed(
        envbox_core::DnsMode::VirtualView,
        true,
        vec![
            DnsUpstream::Tcp {
                address: "127.0.0.1".parse().unwrap(),
                port: 15353,
            },
            DnsUpstream::Udp {
                address: "127.0.0.1".parse().unwrap(),
                port: 15354,
            },
        ],
    );
    dns.validate_runtime_support().unwrap();
    assert!(dns.servers.is_empty());
    assert!(dns
        .flat_fields()
        .unwrap()
        .contains(&("dns_upstream_0_port".into(), "15353".into())));
}

#[test]
fn doh_revocation_policy_defaults_and_strict_roundtrip_are_independent_of_dns_routing() {
    let wire = "mode='virtual_view'\nstrict=true\n[[upstreams]]\ntype='doh'\nurl='https://1.1.1.1/dns-query'\n";
    let standard: DnsProfile = toml::from_str(wire).unwrap();
    assert!(matches!(
        standard.upstreams[0],
        DnsUpstream::Doh {
            tls_revocation: envbox_core::DnsTlsRevocation::Standard,
            ..
        }
    ));
    let strict: DnsProfile =
        toml::from_str(&format!("{wire}tls_revocation='strict_offline'\n")).unwrap();
    assert!(strict.strict);
    assert_eq!(
        toml::from_str::<DnsProfile>(&toml::to_string(&strict).unwrap()).unwrap(),
        strict
    );
    assert!(strict
        .flat_fields()
        .unwrap()
        .contains(&("dns_upstream_0_tls_revocation".into(), "1".into())));
    assert!(toml::from_str::<DnsProfile>(&format!("{wire}tls_revocation='disabled'\n")).is_err());
}
