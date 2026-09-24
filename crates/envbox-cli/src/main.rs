//! envbox CLI: inspect Applications and Environment Profiles.

use envbox_storage::ConfigStore;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = ConfigStore::new(ConfigStore::default_root());

    match args.first().map(String::as_str) {
        Some("profile") if args.get(1).map(String::as_str) == Some("list") => match store.load_profiles()
        {
            Ok(doc) => {
                if doc.profiles.is_empty() {
                    println!("(no profiles)");
                }
                for profile in &doc.profiles {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        profile.id,
                        profile.name,
                        profile.locale.locale_name,
                        profile.locale.region,
                        profile.timezone.windows_id
                    );
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Some("app") if args.get(1).map(String::as_str) == Some("list") => {
            match store.load_applications() {
                Ok(doc) => {
                    if doc.applications.is_empty() {
                        println!("(no applications)");
                    }
                    for app in &doc.applications {
                        println!("{}\t{}\t{:?}", app.id, app.name, app.launch);
                    }
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("run") => {
            eprintln!("error: `envbox run` is not available until ticket 03");
            ExitCode::FAILURE
        }
        _ => {
            eprintln!("envbox — process-scoped Windows environment virtualization");
            eprintln!();
            eprintln!("usage:");
            eprintln!("  envbox profile list");
            eprintln!("  envbox app list");
            eprintln!("  envbox run --profile <id> <command>");
            ExitCode::FAILURE
        }
    }
}
