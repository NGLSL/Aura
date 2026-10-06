use envbox_core::IdentityProfile;
use envbox_storage::ConfigStore;
use std::collections::HashSet;
use std::process::ExitCode;
use uuid::Uuid;

/// Input normalization is limited to formats with an unambiguous canonical spelling.
pub fn set_field(
    identity: &mut IdentityProfile,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let value = value.map(|value| match key {
        "mac_address" => value.to_ascii_uppercase(),
        "machine_guid" => value.to_ascii_lowercase(),
        _ => value.to_owned(),
    });
    match key {
        "computer_name" => identity.computer_name = value,
        "user_name" => identity.user_name = value,
        "mac_address" => identity.mac_address = value,
        "machine_guid" => identity.machine_guid = value,
        _ => return Err(format!("unknown identity field {key:?}")),
    }
    Ok(())
}

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
        .ok_or("identity requires show|set|reset")?;
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
    match action {
        "show" if args.len() == 2 => {
            for (key, value) in [
                ("computer_name", &profile.identity.computer_name),
                ("user_name", &profile.identity.user_name),
                ("mac_address", &profile.identity.mac_address),
                ("machine_guid", &profile.identity.machine_guid),
            ] {
                println!("{key}={}", value.as_deref().unwrap_or("(host)"));
            }
            println!("# supported Win32 read APIs only; real account, permissions and network adapter are unchanged");
            return Ok(());
        }
        "reset" if args.len() == 2 => profile.identity = IdentityProfile::default(),
        "set" if args.len() > 2 => {
            let mut seen = HashSet::new();
            for pair in args[2..].chunks(2) {
                let value = pair.get(1).ok_or("identity option requires a value")?;
                let (key, value) = if pair[0] == "--clear" {
                    (value.as_str(), None)
                } else {
                    let key = match pair[0].as_str() {
                        "--computer-name" => "computer_name",
                        "--user-name" => "user_name",
                        "--mac-address" => "mac_address",
                        "--machine-guid" => "machine_guid",
                        other => return Err(format!("unknown identity option {other:?}")),
                    };
                    (key, Some(value.as_str()))
                };
                if !seen.insert(key.to_owned()) {
                    return Err(format!("duplicate identity field {key}"));
                }
                set_field(&mut profile.identity, key, value)?;
            }
        }
        _ => return Err("identity requires show UUID | set UUID OPTIONS | reset UUID".into()),
    }
    profile.validate().map_err(|err| err.to_string())?;
    store
        .save_profiles(&document)
        .map_err(|err| err.to_string())
}
