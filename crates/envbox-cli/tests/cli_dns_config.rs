use std::process::{Command, Output};
fn run(root: &std::path::Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_envbox"));
    command.env("ENVBOX_CONFIG_ROOT", root).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().unwrap()
}

#[test]
fn typed_dns_cli_order_roundtrip_and_failed_edits_keep_profile() {
    let root = std::env::temp_dir().join(format!("aura-typed-dns-{}", uuid::Uuid::new_v4()));
    let created = run(
        &root,
        &[
            "profile",
            "add",
            "--name",
            "Typed",
            "--locale",
            "en-US",
            "--ui-language",
            "en-US",
            "--region",
            "US",
            "--tz-windows",
            "Pacific Standard Time",
            "--tz-iana",
            "America/Los_Angeles",
            "--dns-mode",
            "virtual_view",
            "--dns",
            "1.1.1.1",
            "--dns",
            "1.0.0.1",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let id = String::from_utf8(created.stdout).unwrap().trim().to_owned();
    let store = envbox_storage::ConfigStore::new(&root);
    let old = store.load_profiles().unwrap().profiles[0].dns.clone();
    assert!(old.strict);
    assert_eq!(old.effective_upstreams().len(), 2);
    assert!(!std::fs::read_to_string(store.profiles_path())
        .unwrap()
        .contains("servers ="));
    for args in [
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "tcp",
            "--address",
            "1.1.1.1",
            "--port",
            "5353",
        ],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "dot",
            "--address",
            "1.1.1.1",
            "--server-name",
            "dns.example",
        ],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "doh",
            "--url",
            "https://dns.example/dns-query",
            "--tls-revocation",
            "strict_offline",
            "--bootstrap",
            "1.1.1.1",
            "--bootstrap",
            "1.0.0.1",
        ],
        vec!["profile", "dns", "move", &id, "--from", "4", "--to", "0"],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "doh",
            "--url",
            "https://dns.example/dns-query",
        ],
    ] {
        let output = run(&root, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let dns = store.load_profiles().unwrap().profiles[0].dns.clone();
    assert!(matches!(
        dns.upstreams[0],
        envbox_core::DnsUpstream::Doh {
            tls_revocation: envbox_core::DnsTlsRevocation::StrictOffline,
            ..
        }
    ));
    assert_eq!(dns.upstreams.len(), 6);
    assert!(matches!(
        &dns.upstreams[5],
        envbox_core::DnsUpstream::Doh { url, bootstrap_ips, .. }
            if url == "https://dns.example/dns-query" && bootstrap_ips.is_empty()
    ));
    assert!(dns.servers.is_empty());
    let shown = run(&root, &["profile", "dns", "show", &id]);
    assert!(shown.status.success());
    assert!(String::from_utf8_lossy(&shown.stdout).contains("runtime_supported = true"));
    let decoded: envbox_core::DnsProfile =
        toml::from_str(&String::from_utf8(shown.stdout).unwrap()).unwrap();
    assert_eq!(decoded, dns);
    let before = std::fs::read(store.profiles_path()).unwrap();
    for args in [
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "doh",
            "--url",
            "http://dns.example/dns-query",
        ],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "udp",
            "--address",
            "1.1.1.1",
            "--port",
            "0",
        ],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "tcp",
            "--address",
            "resolver.example",
        ],
        vec!["profile", "dns", "set", &id, "--strict", "invalid"],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "doh",
            "--url",
            "https://1.1.1.1/dns-query",
            "--tls-revocation",
            "invalid",
        ],
        vec![
            "profile",
            "dns",
            "add",
            &id,
            "--type",
            "udp",
            "--address",
            "1.1.1.1",
            "--url",
            "https://wrong.example",
        ],
    ] {
        assert!(!run(&root, &args).status.success());
        assert_eq!(std::fs::read(store.profiles_path()).unwrap(), before);
    }
    assert!(
        run(&root, &["profile", "dns", "set", &id, "--strict", "false"])
            .status
            .success()
    );
    assert!(!store.load_profiles().unwrap().profiles[0].dns.strict);
    assert!(
        run(&root, &["profile", "dns", "set", &id, "--mode", "host"])
            .status
            .success()
    );
    let host = store.load_profiles().unwrap().profiles[0].dns.clone();
    assert_eq!(host.mode, envbox_core::DnsMode::Host);
    assert!(!host.strict);
    std::fs::remove_dir_all(root).unwrap();
}
