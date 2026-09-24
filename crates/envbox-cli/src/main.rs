//! envbox CLI: define and inspect Applications and Environment Profiles.

use envbox_core::{
    Application, DnsMode, DnsProfile, EnvironmentProfile, LaunchTarget, LocaleProfile,
    RegistryProfile, TimezoneProfile,
};
use envbox_storage::{validate_application, validate_profile, ConfigStore};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use uuid::Uuid;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = ConfigStore::new(ConfigStore::default_root());

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
        Some("run") => cmd_run(&store, &args[1..]),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("envbox — process-scoped Windows environment virtualization");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  envbox profile list");
    eprintln!("  envbox profile add --name N --locale L --ui-language U --region R \\");
    eprintln!("     --tz-windows W --tz-iana I [--dns-mode host|virtual_view] \\");
    eprintln!("     [--dns IP]... [--env K=V]...");
    eprintln!("  envbox app list");
    eprintln!("  envbox app add --name N (--command C | --executable P) --profile ID \\");
    eprintln!("     [--working-directory D] [--arg A]... [--inherit-children] [--audit]");
    eprintln!("  envbox run --profile <id> [--working-directory D] [--no-inherit-children] [--audit] [--] <command> [args...]");
    ExitCode::FAILURE
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

    let request = envbox_launcher::LaunchRequest {
        launch: envbox_core::LaunchTarget::Command {
            command: command.clone(),
        },
        arguments: command_args.to_vec(),
        working_directory,
        profile: Some(profile.clone()),
        instance_id: Uuid::new_v4(),
        inherit_children: !no_inherit,
        audit,
    };

    match envbox_launcher::launch(request) {
        Ok(mut child) => {
            eprintln!(
                "envbox: started pid={} instance={} profile={} (Runtime + core hooks active)",
                child.pid, child.instance_id, child.profile_id
            );
            match child.wait() {
                Ok(status) => {
                    if status.success() {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(status.code().unwrap_or(1) as u8)
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

fn cmd_profile_list(store: &ConfigStore) -> ExitCode {
    match store.load_profiles() {
        Ok(doc) => {
            if doc.profiles.is_empty() {
                println!("(no profiles)");
            }
            for profile in &doc.profiles {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    profile.id,
                    profile.name,
                    profile.locale.locale_name,
                    profile.locale.region,
                    profile.timezone.windows_id,
                    profile.dns.mode_label()
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

    let launch = if !executable.is_empty() {
        LaunchTarget::Executable {
            path: PathBuf::from(executable),
        }
    } else if !command.is_empty() {
        LaunchTarget::Command { command }
    } else {
        eprintln!("error: provide --command or --executable");
        return ExitCode::FAILURE;
    };

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
