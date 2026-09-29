//! envbox CLI: define and inspect Applications and Environment Profiles.

use envbox_core::{
    Application, AuditEvent, BrowserPrivacyProfile, DnsMode, DnsProfile, EnvironmentProfile,
    LaunchTarget, LocaleProfile, RegistryProfile, TimezoneProfile, WebRtcPolicy,
};
use envbox_storage::{validate_application, validate_profile, ConfigStore};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use uuid::Uuid;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store =
        ConfigStore::new(terminal_store_root(&args).unwrap_or_else(ConfigStore::default_root));

    match args.first().map(String::as_str) {
        Some("profile") => match args.get(1).map(String::as_str) {
            Some("list") => cmd_profile_list(&store),
            Some("add") => cmd_profile_add(&store, &args[2..]),
            _ => usage(),
        },
        Some("app") => match args.get(1).map(String::as_str) {
            Some("list") => cmd_app_list(&store),
            Some("add") => cmd_app_add(&store, &args[2..]),
            _ => usage(),
        },
        Some("audit") => match args.get(1).map(String::as_str) {
            Some("show") => cmd_audit_show(&store, &args[2..]),
            Some("export") => cmd_audit_export(&store, &args[2..]),
            _ => usage(),
        },
        Some("run") => cmd_run(&store, &args[1..]),
        Some("hidden") if args.get(1).map(String::as_str) == Some("terminal-run") => {
            cmd_terminal_run(&store, &args[2..])
        }
        Some("terminal-run") => cmd_terminal_run(&store, &args[1..]),
        _ => usage(),
    }
}

/// Windows Terminal's server may have been started before the GUI and does
/// not reliably inherit the GUI's custom `ENVBOX_CONFIG_ROOT`. Pass the
/// already selected store root as a controlled handoff value so terminal-run
/// reads exactly the same persisted Application/Profile documents.
fn terminal_store_root(args: &[String]) -> Option<PathBuf> {
    let is_terminal_run = matches!(
        args.first().map(String::as_str),
        Some("terminal-run" | "hidden")
    );
    if !is_terminal_run {
        return None;
    }
    args.windows(2)
        .find(|pair| pair[0] == "--config-root")
        .map(|pair| PathBuf::from(&pair[1]))
}

fn usage() -> ExitCode {
    eprintln!("envbox — process-scoped Windows environment virtualization");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  envbox profile list");
    eprintln!("  envbox profile add --name N --locale L --ui-language U --region R \\");
    eprintln!("     --tz-windows W --tz-iana I [--dns-mode host|virtual_view] \\");
    eprintln!(
        "     [--dns IP]... [--env K=V]... [--webrtc host|public_interface_only|proxy_only|strict]"
    );
    eprintln!("  envbox app list");
    eprintln!("  envbox app add --name N (--command C | --executable P) --profile ID \\");
    eprintln!("     [--working-directory D] [--arg A]... [--inherit-children] [--audit]");
    eprintln!("  envbox run --profile <id> [--working-directory D] [--no-inherit-children] [--audit] [--] <command> [args...]");
    eprintln!("  envbox hidden terminal-run --config-root ROOT --app-id ID --profile-id ID|host --instance-id ID --job-name NAME");
    eprintln!("  envbox audit show <instance_id> [--summary]");
    eprintln!("  envbox audit export [--out PATH]");
    ExitCode::FAILURE
}

fn parse_instance_id(raw: &str) -> Option<Uuid> {
    let trimmed = raw.trim();
    if let Ok(id) = Uuid::parse_str(trimmed) {
        return Some(id);
    }
    // Accept simple hex (32 chars) as well.
    Uuid::try_parse(trimmed).ok().or_else(|| {
        let simple: String = trimmed.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if simple.len() == 32 {
            Uuid::parse_str(&simple).ok()
        } else {
            None
        }
    })
}

fn cmd_audit_show(store: &ConfigStore, args: &[String]) -> ExitCode {
    let mut instance_raw = String::new();
    let mut summary = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--summary" => summary = true,
            other if other.starts_with("--") => {
                eprintln!("error: unknown flag {other:?}");
                return ExitCode::FAILURE;
            }
            other => {
                if !instance_raw.is_empty() {
                    eprintln!("error: unexpected argument {other:?}");
                    return ExitCode::FAILURE;
                }
                instance_raw = other.to_string();
            }
        }
        i += 1;
    }
    if instance_raw.is_empty() {
        eprintln!("error: audit show requires <instance_id>");
        return ExitCode::FAILURE;
    }
    let Some(instance_id) = parse_instance_id(&instance_raw) else {
        eprintln!("error: invalid instance id {instance_raw:?}");
        return ExitCode::FAILURE;
    };
    let path = store.audit_path(&instance_id);
    if !path.is_file() {
        eprintln!(
            "error: audit file not found for instance {instance_id}: {}",
            path.display()
        );
        return ExitCode::FAILURE;
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let mut events = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match AuditEvent::parse_json_line(line) {
            Ok(ev) => events.push(ev),
            Err(err) => {
                eprintln!(
                    "error: invalid audit line {} in {}: {err}",
                    lineno + 1,
                    path.display()
                );
                return ExitCode::FAILURE;
            }
        }
    }
    if summary {
        println!("instance\t{}", instance_id);
        println!("events\t{}", events.len());
        let mut by_api: HashMap<String, (usize, usize)> = HashMap::new();
        for ev in &events {
            let slot = by_api.entry(ev.api.clone()).or_insert((0, 0));
            let n = ev.n.max(1) as usize;
            slot.0 += n;
            if ev.virtualized {
                slot.1 += n;
            }
        }
        let mut keys: Vec<_> = by_api.into_iter().collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        println!("api\tcalls\tvirtualized");
        for (api, (calls, virt)) in keys {
            println!("{api}\t{calls}\t{virt}");
        }
    } else {
        for ev in &events {
            match ev.to_json_line() {
                Ok(line) => println!("{line}"),
                Err(err) => {
                    eprintln!("error: serialize failed: {err}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn cmd_audit_export(store: &ConfigStore, args: &[String]) -> ExitCode {
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                match args.get(i) {
                    Some(p) if !p.is_empty() => out = Some(PathBuf::from(p)),
                    _ => {
                        eprintln!("error: --out requires a path");
                        return ExitCode::FAILURE;
                    }
                }
            }
            other if other.starts_with("--") => {
                eprintln!("error: unknown flag {other:?}");
                return ExitCode::FAILURE;
            }
            other => {
                eprintln!("error: unexpected argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let dir = store.audit_dir();
    if !dir.is_dir() {
        eprintln!("error: audit directory not found: {}", dir.display());
        return ExitCode::FAILURE;
    }
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
            .collect(),
        Err(err) => {
            eprintln!("error: cannot list {}: {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };
    files.sort();
    if files.is_empty() {
        eprintln!("error: no audit files in {}", dir.display());
        return ExitCode::FAILURE;
    }

    // Merge all instance files. Valid lines only; invalid lines fail closed.
    let mut merged = String::new();
    for path in &files {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        };
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match AuditEvent::parse_json_line(line) {
                Ok(ev) => match ev.to_json_line() {
                    Ok(out_line) => {
                        merged.push_str(&out_line);
                        merged.push('\n');
                    }
                    Err(err) => {
                        eprintln!("error: serialize failed in {}: {err}", path.display());
                        return ExitCode::FAILURE;
                    }
                },
                Err(err) => {
                    eprintln!(
                        "error: invalid audit line {} in {}: {err}",
                        lineno + 1,
                        path.display()
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    match out {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            match std::fs::write(&path, &merged) {
                Ok(()) => {
                    println!("{}", path.display());
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("error: cannot write {}: {err}", path.display());
                    ExitCode::FAILURE
                }
            }
        }
        None => {
            print!("{merged}");
            ExitCode::SUCCESS
        }
    }
}

fn cmd_run(store: &ConfigStore, args: &[String]) -> ExitCode {
    let mut profile_raw = String::new();
    let mut working_directory: Option<PathBuf> = None;
    let mut no_inherit = false;
    let mut audit = false;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match key {
            "--profile" => profile_raw = take(&mut i).unwrap_or_default(),
            "--working-directory" => working_directory = take(&mut i).map(PathBuf::from),
            "--no-inherit-children" => no_inherit = true,
            "--inherit-children" => no_inherit = false,
            "--audit" => audit = true,
            "--" => {
                rest.extend_from_slice(&args[i + 1..]);
                break;
            }
            other if other.starts_with("--") => {
                eprintln!("error: unknown flag {other:?}");
                return ExitCode::FAILURE;
            }
            _ => {
                rest.extend_from_slice(&args[i..]);
                break;
            }
        }
        i += 1;
    }

    if profile_raw.is_empty() {
        eprintln!("error: --profile <id-or-name> is required");
        return ExitCode::FAILURE;
    }

    // Startup Fail Policy: missing/corrupt profile must not start.
    let profile = match store.load_profiles() {
        Ok(doc) => doc.profiles.into_iter().find(|p| {
            p.id.to_string() == profile_raw
                || p.id.simple().to_string() == profile_raw
                || p.name.eq_ignore_ascii_case(&profile_raw)
        }),
        Err(err) => {
            eprintln!("error: profiles store unreadable: {err}");
            return ExitCode::FAILURE;
        }
    };
    let Some(profile) = profile else {
        eprintln!("error: profile {profile_raw:?} not found");
        return ExitCode::FAILURE;
    };
    if let Err(err) = envbox_storage::validate_profile(&profile) {
        eprintln!("error: {err}");
        return ExitCode::FAILURE;
    }

    let Some((command, command_args)) = rest.split_first() else {
        eprintln!("error: missing command to run");
        return ExitCode::FAILURE;
    };

    // AUMID / WindowsApps paths must activate as Packaged (never CreateProcess).
    let launch = if envbox_launcher::extract_aumid(command).is_some()
        || envbox_launcher::is_windows_apps_path(command)
    {
        envbox_launcher::launch_target_from_user_path(command)
    } else {
        envbox_core::LaunchTarget::Command {
            command: command.clone(),
        }
    };

    let request = envbox_launcher::SessionStartRequest {
        application_id: Uuid::nil(),
        launch,
        arguments: command_args.to_vec(),
        working_directory,
        profile: Some(profile.clone()),
        inherit_children: !no_inherit,
        audit,
    };

    match envbox_launcher::start_session(request) {
        Ok(mut handle) => {
            eprintln!(
                "envbox: started pid={} instance={} profile={} (Runtime + core hooks active)",
                handle.instance.root_pid, handle.instance.id, handle.instance.profile_id
            );
            if matches!(
                envbox_core::BrowserEngine::from_image(command),
                envbox_core::BrowserEngine::Chromium | envbox_core::BrowserEngine::Edge
            ) {
                eprintln!(
                    "envbox: Chrome/Edge sandboxed renderers have partial native environment coverage"
                );
            }
            match handle.wait_root() {
                Ok(code) => {
                    if code == 0 {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(code.clamp(0, 255) as u8)
                    }
                }
                Err(err) => {
                    eprintln!("error: wait failed: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Windows Terminal handoff entry point. The actual command and arguments are
/// deliberately loaded from the persisted Application record, so `wt.exe`
/// never receives user command text and cannot accidentally start an
/// unvirtualized copy of a CLI.
fn cmd_terminal_run(store: &ConfigStore, args: &[String]) -> ExitCode {
    let mut app_raw = None;
    let mut profile_raw = None;
    let mut instance_raw = None;
    let mut job_name = None;
    let mut config_fingerprint = None;
    let mut i = 0;
    while i < args.len() {
        let value = |index: &mut usize| -> Option<String> {
            *index += 1;
            args.get(*index).cloned()
        };
        match args[i].as_str() {
            "--config-root" => {
                if value(&mut i).is_none() {
                    eprintln!("error: terminal-run --config-root requires a path");
                    return ExitCode::FAILURE;
                }
            }
            "--config-fingerprint" => {
                let Some(raw) = value(&mut i) else {
                    eprintln!("error: terminal-run --config-fingerprint requires a value");
                    return ExitCode::FAILURE;
                };
                config_fingerprint = match raw.parse::<u64>() {
                    Ok(value) => Some(value),
                    Err(_) => {
                        eprintln!("error: invalid terminal-run config fingerprint");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--app-id" => app_raw = value(&mut i),
            "--profile-id" => profile_raw = value(&mut i),
            "--instance-id" => instance_raw = value(&mut i),
            "--job-name" => job_name = value(&mut i),
            other => {
                eprintln!("error: terminal-run unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let Some(app_raw) = app_raw.filter(|value| !value.is_empty()) else {
        eprintln!("error: terminal-run requires --app-id");
        return ExitCode::FAILURE;
    };
    let Some(profile_raw) = profile_raw.filter(|value| !value.is_empty()) else {
        eprintln!("error: terminal-run requires --profile-id");
        return ExitCode::FAILURE;
    };
    let Some(instance_raw) = instance_raw.filter(|value| !value.is_empty()) else {
        eprintln!("error: terminal-run requires --instance-id");
        return ExitCode::FAILURE;
    };
    let Some(job_name) = job_name.filter(|value| !value.is_empty()) else {
        eprintln!("error: terminal-run requires --job-name");
        return ExitCode::FAILURE;
    };

    let Some(application_id) = parse_instance_id(&app_raw) else {
        eprintln!("error: invalid --app-id {app_raw:?}");
        return ExitCode::FAILURE;
    };
    let Some(instance_id) = parse_instance_id(&instance_raw) else {
        eprintln!("error: invalid --instance-id {instance_raw:?}");
        return ExitCode::FAILURE;
    };
    if !job_name.starts_with(r"Local\Aura-") || job_name.contains('\0') {
        eprintln!("error: invalid terminal-run Job name");
        return ExitCode::FAILURE;
    }
    if let Some(expected) = config_fingerprint {
        let actual = terminal_config_fingerprint(store.root());
        if actual != expected {
            eprintln!("error: application/profile store changed before Windows Terminal handoff");
            return ExitCode::FAILURE;
        }
    }
    let marker = envbox_launcher::terminal_root_marker_path(instance_id);
    let cancel_marker = envbox_launcher::terminal_cancel_marker_path(instance_id);
    if terminal_handoff_cancelled(&cancel_marker) {
        eprintln!("error: Windows Terminal handoff was stopped before activation");
        return ExitCode::FAILURE;
    }

    let app = match store.load_applications() {
        Ok(doc) => doc
            .applications
            .into_iter()
            .find(|app| app.id == application_id),
        Err(err) => {
            eprintln!("error: applications store unreadable: {err}");
            return ExitCode::FAILURE;
        }
    };
    let Some(app) = app else {
        eprintln!("error: application {application_id} not found");
        return ExitCode::FAILURE;
    };
    if let Err(err) = validate_application(&app) {
        eprintln!("error: saved application is invalid: {err}");
        return ExitCode::FAILURE;
    }
    if app.console_host != envbox_core::ConsoleHost::WindowsTerminal {
        eprintln!("error: application is no longer configured for Windows Terminal");
        return ExitCode::FAILURE;
    }

    let profile = if profile_raw.eq_ignore_ascii_case("host") {
        None
    } else {
        let Some(profile_id) = parse_instance_id(&profile_raw) else {
            eprintln!("error: invalid --profile-id {profile_raw:?}");
            return ExitCode::FAILURE;
        };
        match store.load_profiles() {
            Ok(doc) => match doc
                .profiles
                .into_iter()
                .find(|profile| profile.id == profile_id)
            {
                Some(profile) => Some(profile),
                None => {
                    eprintln!("error: profile {profile_id} not found");
                    return ExitCode::FAILURE;
                }
            },
            Err(err) => {
                eprintln!("error: profiles store unreadable: {err}");
                return ExitCode::FAILURE;
            }
        }
    };

    let request = envbox_launcher::SessionStartRequest {
        application_id,
        launch: app.launch,
        arguments: app.arguments,
        working_directory: app.working_directory,
        profile,
        inherit_children: app.inherit_children,
        audit: app.audit,
    };
    match envbox_launcher::start_session_in_named_job(request, instance_id, &job_name) {
        Ok(mut handle) => {
            let root_pid = handle.instance.root_pid;
            if terminal_handoff_cancelled(&cancel_marker) {
                let _ = handle.job.as_mut().map(|job| job.terminate());
                eprintln!("error: Windows Terminal handoff was stopped during activation");
                return ExitCode::FAILURE;
            }
            if let Err(err) = std::fs::write(&marker, format!("{root_pid}\nrunning\n")) {
                let _ = handle.job.as_mut().map(|job| job.terminate());
                eprintln!("error: terminal-run cannot publish root PID: {err}");
                return ExitCode::FAILURE;
            }
            if terminal_handoff_cancelled(&cancel_marker) {
                let _ = handle.job.as_mut().map(|job| job.terminate());
                let _ = std::fs::write(&marker, format!("{root_pid}\nexited\n"));
                eprintln!("error: Windows Terminal handoff was stopped before wait");
                return ExitCode::FAILURE;
            }
            let result = match handle.wait_root() {
                Ok(code) if code == 0 => ExitCode::SUCCESS,
                Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
                Err(err) => {
                    eprintln!("error: terminal-run wait failed: {err}");
                    ExitCode::FAILURE
                }
            };
            // Keep the marker long enough for the GUI refresh loop to observe
            // a fast CLI exit; the GUI owns cleanup of the per-instance file.
            let _ = std::fs::write(&marker, format!("{root_pid}\nexited\n"));
            result
        }
        Err(err) => {
            eprintln!("error: terminal-run failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn terminal_handoff_cancelled(marker: &std::path::Path) -> bool {
    std::fs::read_to_string(marker)
        .map(|text| text.trim() == "cancelled")
        .unwrap_or(false)
}

fn terminal_config_fingerprint(root: &std::path::Path) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for name in ["applications.toml", "profiles.toml"] {
        for byte in name.as_bytes().iter().copied().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        if let Ok(bytes) = std::fs::read(root.join(name)) {
            for byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

fn cmd_profile_list(store: &ConfigStore) -> ExitCode {
    match store.load_profiles() {
        Ok(doc) => {
            if doc.profiles.is_empty() {
                println!("(no profiles)");
            }
            for profile in &doc.profiles {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\twebrtc={}",
                    profile.id,
                    profile.name,
                    profile.locale.locale_name,
                    profile.locale.region,
                    profile.timezone.windows_id,
                    profile.dns.mode_label(),
                    profile.browser.webrtc.as_str(),
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_app_list(store: &ConfigStore) -> ExitCode {
    match store.load_applications() {
        Ok(doc) => {
            if doc.applications.is_empty() {
                println!("(no applications)");
            }
            for app in &doc.applications {
                let launch = match &app.launch {
                    LaunchTarget::Executable { path } => format!("exe={}", path.display()),
                    LaunchTarget::Command { command } => format!("cmd={command}"),
                    LaunchTarget::Packaged { aumid, .. } => format!("packaged={aumid}"),
                };
                println!(
                    "{}\t{}\t{}\tprofile={}",
                    app.id, app.name, launch, app.default_profile_id
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_profile_add(store: &ConfigStore, args: &[String]) -> ExitCode {
    let mut name = String::new();
    let mut locale_name = String::new();
    let mut ui_language = String::new();
    let mut region = String::new();
    let mut tz_windows = String::new();
    let mut tz_iana = String::new();
    let mut dns_mode = DnsMode::Host;
    let mut servers: Vec<IpAddr> = Vec::new();
    let mut environment = HashMap::new();
    let mut webrtc = WebRtcPolicy::Host;

    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match key {
            "--name" => name = take(&mut i).unwrap_or_default(),
            "--locale" => locale_name = take(&mut i).unwrap_or_default(),
            "--ui-language" => ui_language = take(&mut i).unwrap_or_default(),
            "--region" => region = take(&mut i).unwrap_or_default(),
            "--tz-windows" => tz_windows = take(&mut i).unwrap_or_default(),
            "--tz-iana" => tz_iana = take(&mut i).unwrap_or_default(),
            "--webrtc" => {
                let raw = take(&mut i).unwrap_or_default();
                match WebRtcPolicy::parse(&raw) {
                    Some(p) => webrtc = p,
                    None => {
                        eprintln!(
                            "error: unknown webrtc policy {raw:?} \
                             (host|public_interface_only|proxy_only|strict)"
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--dns-mode" => {
                let mode = take(&mut i).unwrap_or_default();
                dns_mode = match mode.as_str() {
                    "host" => DnsMode::Host,
                    "virtual_view" | "virtualview" | "custom" => DnsMode::VirtualView,
                    other => {
                        eprintln!("error: unknown dns-mode {other:?} (host|virtual_view)");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--dns" => {
                let raw = take(&mut i).unwrap_or_default();
                match raw.parse::<IpAddr>() {
                    Ok(ip) => servers.push(ip),
                    Err(_) => {
                        eprintln!("error: invalid DNS address {raw:?}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--env" => {
                let raw = take(&mut i).unwrap_or_default();
                match raw.split_once('=') {
                    Some((k, v)) => {
                        environment.insert(k.to_string(), v.to_string());
                    }
                    None => {
                        eprintln!("error: --env expects KEY=VALUE, got {raw:?}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            other => {
                eprintln!("error: unknown flag {other:?}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let profile = EnvironmentProfile {
        id: Uuid::new_v4(),
        name,
        locale: LocaleProfile {
            locale_name,
            ui_language,
            region,
        },
        timezone: TimezoneProfile {
            windows_id: tz_windows,
            iana_id: tz_iana,
        },
        dns: DnsProfile {
            mode: dns_mode,
            servers,
        },
        environment,
        registry: RegistryProfile::default(),
        browser: BrowserPrivacyProfile { webrtc },
    };

    if let Err(err) = validate_profile(&profile) {
        eprintln!("error: {err}");
        return ExitCode::FAILURE;
    }

    if store.ensure_dirs().is_err() {
        eprintln!("error: cannot create config directory");
        return ExitCode::FAILURE;
    }
    let mut doc = match store.load_profiles() {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!("error: cannot load profiles.toml: {err}");
            return ExitCode::FAILURE;
        }
    };
    doc.profiles.push(profile.clone());
    match store.save_profiles(&doc) {
        Ok(()) => {
            println!("{}", profile.id);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_app_add(store: &ConfigStore, args: &[String]) -> ExitCode {
    let mut name = String::new();
    let mut command = String::new();
    let mut executable = String::new();
    let mut profile_raw = String::new();
    let mut working_directory: Option<PathBuf> = None;
    let mut arguments: Vec<String> = Vec::new();
    let mut inherit_children = true;
    let mut audit = false;

    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match key {
            "--name" => name = take(&mut i).unwrap_or_default(),
            "--command" => command = take(&mut i).unwrap_or_default(),
            "--executable" => executable = take(&mut i).unwrap_or_default(),
            "--profile" => profile_raw = take(&mut i).unwrap_or_default(),
            "--working-directory" => {
                working_directory = take(&mut i).map(PathBuf::from);
            }
            "--arg" => {
                if let Some(v) = take(&mut i) {
                    arguments.push(v);
                }
            }
            "--inherit-children" => inherit_children = true,
            "--no-inherit-children" => inherit_children = false,
            "--audit" => audit = true,
            other => {
                eprintln!("error: unknown flag {other:?}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let launch_raw = if !executable.is_empty() {
        LaunchTarget::Executable {
            path: PathBuf::from(executable),
        }
    } else if !command.is_empty() {
        LaunchTarget::Command { command }
    } else {
        eprintln!("error: provide --command or --executable");
        return ExitCode::FAILURE;
    };
    // AUMID / WindowsApps / execution-alias must become Packaged.
    let launch = envbox_launcher::normalize_launch_target(&launch_raw);

    let default_profile_id = match profile_raw.parse::<Uuid>() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("error: --profile must be a UUID, got {profile_raw:?}");
            return ExitCode::FAILURE;
        }
    };

    // Default profile must already exist (stable id contract).
    match store.load_profiles() {
        Ok(doc) if doc.profiles.iter().any(|p| p.id == default_profile_id) => {}
        Ok(_) => {
            eprintln!("error: profile {default_profile_id} not found");
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    }

    let app = Application {
        id: Uuid::new_v4(),
        name,
        launch,
        console_host: envbox_core::ConsoleHost::Direct,
        working_directory,
        arguments,
        default_profile_id,
        inherit_children,
        audit,
    };

    if let Err(err) = validate_application(&app) {
        eprintln!("error: {err}");
        return ExitCode::FAILURE;
    }

    if store.ensure_dirs().is_err() {
        eprintln!("error: cannot create config directory");
        return ExitCode::FAILURE;
    }
    let mut doc = match store.load_applications() {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!("error: cannot load applications.toml: {err}");
            return ExitCode::FAILURE;
        }
    };
    doc.applications.push(app.clone());
    match store.save_applications(&doc) {
        Ok(()) => {
            println!("{}", app.id);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

trait DnsModeLabel {
    fn mode_label(&self) -> &'static str;
}

impl DnsModeLabel for DnsProfile {
    fn mode_label(&self) -> &'static str {
        match self.mode {
            DnsMode::Host => "host",
            DnsMode::VirtualView => "virtual_view",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_store_root_uses_explicit_handoff_root() {
        let args = vec![
            "hidden".into(),
            "terminal-run".into(),
            "--config-root".into(),
            r"D:\Aura\config".into(),
            "--app-id".into(),
            Uuid::new_v4().to_string(),
        ];
        assert_eq!(
            terminal_store_root(&args),
            Some(PathBuf::from(r"D:\Aura\config"))
        );
    }

    #[test]
    fn terminal_config_fingerprint_changes_when_store_changes() {
        let root = std::env::temp_dir().join(format!("envbox-cli-store-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let before = terminal_config_fingerprint(&root);
        std::fs::write(root.join("applications.toml"), b"[applications]\n").unwrap();
        let after = terminal_config_fingerprint(&root);
        assert_ne!(before, after);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_handoff_cancelled_requires_exact_marker() {
        let marker = std::env::temp_dir().join(format!("envbox-cli-marker-{}", Uuid::new_v4()));
        std::fs::write(&marker, "cancelled\n").unwrap();
        assert!(terminal_handoff_cancelled(&marker));
        std::fs::write(&marker, "1234\nrunning\n").unwrap();
        assert!(!terminal_handoff_cancelled(&marker));
        let _ = std::fs::remove_file(marker);
    }
}
