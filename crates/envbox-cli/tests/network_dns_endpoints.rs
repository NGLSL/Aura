//! Explicit local NIC sinks exercise endpoint policy without external traffic.
use std::net::{Ipv4Addr, UdpSocket};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

fn cli(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_envbox"));
    command.env("ENVBOX_CONFIG_ROOT", root);
    command
}

fn field<'a>(text: &'a str, key: &str) -> &'a str {
    text.split(key)
        .nth(1)
        .expect("Probe field")
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap()
        .trim()
}

#[test]
#[ignore = "requires fresh uninjected Host and AURA_TEST_INTERFACE_IP from an actual Preferred NIC"]
fn strict_webrtc_udp_allows_only_typed_udp_endpoint() {
    let ip: Ipv4Addr = std::env::var("AURA_TEST_INTERFACE_IP")
        .expect("explicit physical interface IPv4 required")
        .parse()
        .unwrap();
    assert!(
        !ip.is_loopback() && !ip.is_unspecified(),
        "loopback is not deny evidence"
    );
    let dll = std::env::var("ENVBOX_TEST_RUNTIME_DLL").expect("pinned Runtime DLL required");
    let probe = std::env::var("ENVBOX_TEST_PROBE_EXE").expect("actual Probe path required");
    let root = std::env::temp_dir().join(format!("aura-udp-endpoint-{}", Uuid::new_v4()));
    let first = UdpSocket::bind((ip, 0)).expect("bind selected NIC first sink");
    let second = UdpSocket::bind((ip, 0)).expect("bind selected NIC second sink");
    for sink in [&first, &second] {
        sink.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
    }
    let endpoint = first.local_addr().unwrap();
    let other = second.local_addr().unwrap();
    let send = |profile: Option<&str>, target: std::net::SocketAddr| {
        let mut command = match profile {
            Some(id) => {
                let mut command = cli(&root);
                command
                    .env("ENVBOX_RUNTIME_DLL", &dll)
                    .args(["run", "--profile", id])
                    .arg(&probe);
                command
            }
            None => Command::new(&probe),
        };
        let output = command
            .args(["--udp-send-to", &target.to_string()])
            .output()
            .unwrap();
        assert!(output.status.success(), "Probe failed: {output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let receive = |sink: &UdpSocket, expected: bool| {
        let mut buffer = [0; 64];
        match sink.recv_from(&mut buffer) {
            Ok((count, _)) => {
                assert!(expected, "denied endpoint received a packet");
                assert_eq!(&buffer[..count], b"aura-endpoint-fixture");
            }
            Err(error) => {
                assert!(!expected, "allowed packet missing: {error}");
                assert!(matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ));
            }
        }
    };
    for (target, sink) in [(endpoint, &first), (other, &second)] {
        let text = send(None, target);
        assert_eq!(field(&text, "UdpSend_Error:"), "0");
        receive(sink, true);
        eprintln!("native Host {target}: {text}");
    }
    for transport in ["udp", "tcp"] {
        let output = cli(&root)
            .args([
                "profile",
                "add",
                "--name",
                transport,
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
                "host",
                "--webrtc",
                "strict",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        for args in [
            vec![
                "profile",
                "dns",
                "add",
                &id,
                "--type",
                transport,
                "--address",
                &ip.to_string(),
                "--port",
                &endpoint.port().to_string(),
            ],
            vec![
                "profile",
                "dns",
                "set",
                &id,
                "--mode",
                "virtual_view",
                "--strict",
                "true",
            ],
        ] {
            let output = cli(&root).args(args).output().unwrap();
            assert!(output.status.success(), "typed Profile config: {output:?}");
        }
        for (target, sink) in [(endpoint, &first), (other, &second)] {
            let allowed = transport == "udp" && target == endpoint;
            let text = send(Some(&id), target);
            assert_eq!(
                field(&text, "UdpSend_Error:"),
                if allowed { "0" } else { "10013" },
                "{transport} {target}: {text}"
            );
            assert_eq!(
                field(&text, "UdpSend_Bytes:"),
                if allowed { "21" } else { "0" }
            );
            receive(sink, allowed);
            eprintln!("strict typed {transport} {target}: {text}");
        }
    }
}
