use envbox_core::{DnsMode, DnsProfile, DnsUpstream};
use envbox_storage::ConfigStore;
use std::process::ExitCode;
use uuid::Uuid;

pub fn command(store: &ConfigStore, args: &[String]) -> ExitCode {
    match execute(store, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
fn execute(store: &ConfigStore, args: &[String]) -> Result<(), String> {
    let action = args
        .first()
        .map(String::as_str)
        .ok_or("profile dns requires show|add|remove|move|set")?;
    let id: Uuid = args
        .get(1)
        .ok_or("Profile UUID required")?
        .parse()
        .map_err(|_| "invalid Profile UUID")?;
    let mut document = store.load_profiles().map_err(|err| err.to_string())?;
    let profile = document
        .profiles
        .iter_mut()
        .find(|profile| profile.id == id)
        .ok_or("Profile UUID not found")?;
    if action == "show" && args.len() == 2 {
        println!(
            "{}",
            toml::to_string_pretty(&profile.dns).map_err(|err| err.to_string())?
        );
        println!(
            "# runtime_supported = {}",
            profile.dns.validate_runtime_support().is_ok()
        );
        println!(
            "# plaintext_fallback = {}",
            profile
                .dns
                .effective_upstreams()
                .iter()
                .any(DnsUpstream::is_plaintext)
        );
        return Ok(());
    }
    let mut options: std::collections::HashMap<&str, &str> = Default::default();
    let mut bootstrap = Vec::new();
    let mut cursor = 2;
    while cursor < args.len() {
        let key = args[cursor].as_str();
        let value = args
            .get(cursor + 1)
            .ok_or_else(|| format!("{key} requires value"))?
            .as_str();
        if key == "--bootstrap" {
            bootstrap.push(value.parse().map_err(|_| "bootstrap must be literal IP")?);
        } else if options.insert(key, value).is_some() {
            return Err(format!("duplicate flag {key}"));
        }
        cursor += 2;
    }
    let get = |key: &str| -> Result<&str, String> {
        options
            .get(key)
            .copied()
            .ok_or_else(|| format!("{key} required"))
    };
    let mut upstreams = profile.dns.effective_upstreams();
    let allowed: &[&str] = match action {
        "add" => {
            let kind = get("--type")?;
            let upstream = match kind {
                "udp" | "tcp" | "dot" => {
                    let address = get("--address")?
                        .parse()
                        .map_err(|_| "address must be literal IP")?;
                    let port = options
                        .get("--port")
                        .map(|value| value.parse::<u16>())
                        .transpose()
                        .map_err(|_| "port must be 1..65535")?
                        .unwrap_or(if kind == "dot" { 853 } else { 53 });
                    match kind {
                        "udp" => DnsUpstream::Udp { address, port },
                        "tcp" => DnsUpstream::Tcp { address, port },
                        _ => DnsUpstream::Dot {
                            address,
                            port,
                            server_name: get("--server-name")?.into(),
                        },
                    }
                }
                "doh" => DnsUpstream::Doh {
                    url: get("--url")?.into(),
                    bootstrap_ips: bootstrap.clone(),
                    tls_revocation: options
                        .get("--tls-revocation")
                        .copied()
                        .unwrap_or("standard")
                        .parse()?,
                },
                _ => return Err("type must be udp|tcp|dot|doh".into()),
            };
            upstream.validate().map_err(|err| err.to_string())?;
            upstreams.push(upstream);
            match kind {
                "doh" => &["--type", "--url", "--tls-revocation"],
                "dot" => &["--type", "--address", "--port", "--server-name"],
                _ => &["--type", "--address", "--port"],
            }
        }
        "remove" => {
            let index = get("--index")?
                .parse::<usize>()
                .map_err(|_| "invalid index")?;
            if index >= upstreams.len() {
                return Err("upstream index not found".into());
            }
            upstreams.remove(index);
            &["--index"]
        }
        "move" => {
            let from = get("--from")?
                .parse::<usize>()
                .map_err(|_| "invalid source index")?;
            let to = get("--to")?
                .parse::<usize>()
                .map_err(|_| "invalid destination index")?;
            if from >= upstreams.len() || to >= upstreams.len() {
                return Err("upstream index not found".into());
            }
            let item = upstreams.remove(from);
            upstreams.insert(to, item);
            &["--from", "--to"]
        }
        "set" => {
            if let Some(mode) = options.get("--mode") {
                profile.dns.mode = match *mode {
                    "host" => DnsMode::Host,
                    "virtual_view" => DnsMode::VirtualView,
                    _ => return Err("mode must be host|virtual_view".into()),
                };
            }
            if let Some(strict) = options.get("--strict") {
                profile.dns.strict = strict
                    .parse::<bool>()
                    .map_err(|_| "strict must be true|false")?;
            }
            if options.is_empty() {
                return Err("set requires --mode or --strict".into());
            }
            &["--mode", "--strict"]
        }
        _ => return Err("profile dns requires show|add|remove|move|set".into()),
    };
    if options.keys().any(|key| !allowed.contains(key))
        || (!bootstrap.is_empty() && !(action == "add" && options.get("--type") == Some(&"doh")))
    {
        return Err("unexpected DNS flags for this command".into());
    }
    profile.dns = DnsProfile::typed(profile.dns.mode.clone(), profile.dns.strict, upstreams);
    profile.dns.validate().map_err(|err| err.to_string())?;
    store
        .save_profiles(&document)
        .map_err(|err| err.to_string())?;
    println!("saved DNS configuration; unsupported transport/strict combinations cannot launch");
    Ok(())
}
