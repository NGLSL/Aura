use envbox_core::{Container, ContainerMode};
use envbox_storage::ConfigStore;
use std::process::ExitCode;
use uuid::Uuid;

const LEGACY_POLICY_UNSUPPORTED: &str = "container policy is unsupported: environment-information containers do not isolate file or Registry writes; historical storage policy configuration is preserved and inactive";

pub fn reject_legacy_policy() -> ExitCode {
    eprintln!("error: {LEGACY_POLICY_UNSUPPORTED}");
    ExitCode::FAILURE
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
    if matches!(
        args.first().map(String::as_str),
        Some("instances" | "stop" | "stop-all")
    ) {
        return control_command(args);
    }
    if args.first().map(String::as_str) == Some("run") {
        return run_command(store, &args[1..]);
    }
    if matches!(
        args.first().map(String::as_str),
        Some("prepare" | "show-snapshot")
    ) {
        return snapshot_command(store, args);
    }
    if args.first().map(String::as_str) == Some("policy") {
        return Err(LEGACY_POLICY_UNSUPPORTED.into());
    }
    let mut doc = store.load_containers().map_err(|err| err.to_string())?;
    if args.first().map(String::as_str) == Some("list") && args.len() == 1 {
        println!("id\tname\tprofile_id\tmode\tcreated_at_unix_ms");
        for workspace in doc.containers {
            println!(
                "{}\t{}\t{}\tcompatibility\t{}",
                workspace.id, workspace.name, workspace.profile_id, workspace.created_at_unix_ms
            );
        }
        return Ok(());
    }
    let action = args.first().map(String::as_str).unwrap_or("");
    let mut index = if action == "edit" { 2 } else { 1 };
    let mut workspace = match action {
        "create" => Container::new("", Uuid::nil()),
        "edit" => {
            let id = args.get(1).ok_or("edit requires Container UUID")?.parse::<Uuid>().map_err(|_| "invalid Container UUID")?;
            doc.containers.iter().find(|value| value.id == id).cloned().ok_or("Container UUID not found")?
        }
        _ => return Err("usage: envbox container list | create --name N --profile UUID [--mode compatibility] | edit UUID [--name N] [--profile UUID]".into()),
    };
    let mut seen = std::collections::HashSet::new();
    while index < args.len() {
        let flag = &args[index];
        if !seen.insert(flag) {
            return Err(format!("duplicate flag {flag}"));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--name" => workspace.name = value.clone(),
            "--profile" => {
                workspace.profile_id = value.parse().map_err(|_| "Profile must be a UUID")?
            }
            "--mode" => {
                workspace.mode = match value.as_str() {
                    "compatibility" => ContainerMode::Compatibility,
                    "container" | "strong" => return Err("environment-information containers support only compatibility mode; container/strong resource isolation is outside this product scope".into()),
                    _ => return Err(format!("unknown mode {value}")),
                }
            }
            _ => return Err(format!("unknown flag {flag}")),
        }
        index += 2;
    }
    workspace.validate().map_err(|err| err.to_string())?;
    let id = workspace.id;
    if action == "edit" {
        *doc.containers
            .iter_mut()
            .find(|value| value.id == id)
            .unwrap() = workspace;
    } else {
        doc.containers.push(workspace);
    }
    store.save_containers(&doc).map_err(|err| err.to_string())?;
    println!("{id}");
    Ok(())
}

fn control_command(args: &[String]) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = args;
        Err("Supervisor management requires Windows".into())
    }
    #[cfg(windows)]
    {
        use envbox_supervisor::{Request, RunCommand, SupervisorClient, PROTOCOL_VERSION};
        let container_id: Uuid = args
            .get(1)
            .ok_or("Container UUID required")?
            .parse()
            .map_err(|_| "invalid Container UUID")?;
        let mut instance_id = None;
        let mut application_id = None;
        let mut request_id = Uuid::new_v4();
        let mut seen = std::collections::HashSet::new();
        let mut index = 2;
        while index < args.len() {
            let flag = &args[index];
            if !seen.insert(flag) {
                return Err(format!("duplicate flag {flag}"));
            }
            let value: Uuid = args
                .get(index + 1)
                .ok_or_else(|| format!("{flag} requires UUID"))?
                .parse()
                .map_err(|_| "invalid UUID")?;
            match flag.as_str() {
                "--instance" => instance_id = Some(value),
                "--application" => application_id = Some(value),
                "--request" => request_id = value,
                _ => return Err(format!("unknown flag {flag}")),
            }
            index += 2;
        }
        let command = match args[0].as_str() {
            "instances" => "List",
            "stop" => "Stop",
            "stop-all" => "StopAll",
            _ => unreachable!(),
        };
        let run = if command == "Stop" {
            Some(RunCommand {
                container_id,
                instance_id: instance_id.ok_or("--instance UUID required")?,
                application_id: application_id.ok_or("--application UUID required")?,
            })
        } else {
            if instance_id.is_some() || application_id.is_some() {
                return Err("instance/application flags are available only for stop".into());
            }
            None
        };
        let mut client =
            SupervisorClient::beside_current_executable().map_err(|error| error.to_string())?;
        client.timeout = std::time::Duration::from_secs(20);
        let hello = client.ensure_started().map_err(|error| error.to_string())?;
        let response = client
            .request(Request {
                version: PROTOCOL_VERSION,
                generation: Some(hello.generation),
                request_id: request_id.to_string(),
                command: command.into(),
                run,
                container_id: Some(container_id),
            })
            .map_err(|error| error.to_string())?;
        println!(
            "status\t{}\nrequest_id\t{}\ncontainer_id\t{}\nscope\tcurrent_supervisor_generation\nrestart_recovery\tverified_job_and_runtime_only\nrecovery_limit\tunknown_members_or_missing_job_reports_tracking_lost",
            response.status, request_id, container_id
        );
        print_environment_limits();
        println!("instance_id\tapplication_id\tstate\troot_pid\tactive_members\tmode");
        for view in response.instances {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                view.result.instance_id,
                view.result.application_id,
                view.result.state,
                view.result.root_pid,
                view.process_ids.len(),
                view.result.mode
            );
            print_runtime_observations(&view.result);
            if let Some(error) = view.result.error {
                eprintln!("instance {}: {error}", view.result.instance_id);
            }
        }
        if response.status != "Ok" {
            return Err(format!("Supervisor {}", response.status));
        }
        Ok(())
    }
}

fn run_command(store: &ConfigStore, args: &[String]) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        let _ = (store, args);
        Err("Supervisor workspace Run requires Windows".into())
    }
    #[cfg(windows)]
    {
        use envbox_supervisor::{Request, RunCommand, SupervisorClient, PROTOCOL_VERSION};
        let container_id: Uuid = args
            .first()
            .ok_or("Container UUID required")?
            .parse()
            .map_err(|_| "invalid Container UUID")?;
        let mut application_id = None;
        let mut instance_id = Uuid::new_v4();
        let mut request_id = Uuid::new_v4();
        let mut seen = std::collections::HashSet::new();
        let mut cursor = 1;
        while cursor < args.len() {
            let flag = &args[cursor];
            if !seen.insert(flag) {
                return Err(format!("duplicate flag {flag}"));
            }
            let value: Uuid = args
                .get(cursor + 1)
                .ok_or_else(|| format!("{flag} requires UUID"))?
                .parse()
                .map_err(|_| format!("{flag} requires UUID"))?;
            match flag.as_str() {
                "--application" => application_id = Some(value),
                "--instance" => instance_id = value,
                "--request" => request_id = value,
                _ => return Err(format!("unknown flag {flag}")),
            }
            cursor += 2;
        }
        let application_id = application_id
            .ok_or("--application UUID required; arbitrary command paths are not accepted")?;
        if store
            .run_snapshot_path(container_id, instance_id)
            .try_exists()
            .map_err(|error| error.to_string())?
        {
            store
                .load_run_snapshot(container_id, instance_id)
                .map_err(|error| error.to_string())?;
        } else {
            store
                .prepare_run_snapshot(container_id, instance_id)
                .map_err(|error| error.to_string())?;
        }
        let mut client =
            SupervisorClient::beside_current_executable().map_err(|error| error.to_string())?;
        client.timeout = std::time::Duration::from_secs(20);
        let hello = client.ensure_started().map_err(|error| error.to_string())?;
        let response = client
            .request(Request {
                version: PROTOCOL_VERSION,
                generation: Some(hello.generation),
                request_id: request_id.to_string(),
                command: "Run".into(),
                container_id: None,
                run: Some(RunCommand {
                    container_id,
                    instance_id,
                    application_id,
                }),
            })
            .map_err(|error| error.to_string())?;
        let result = response
            .run
            .ok_or_else(|| format!("Supervisor {}", response.status))?;
        println!("status\t{}\nrequest_id\t{}\ncontainer_id\t{}\ninstance_id\t{}\nroot_pid\t{}\nmode\t{}\nentry_guarantee\t{}\nconfiguration_id\t{}", response.status, request_id, result.container_id, result.instance_id, result.root_pid, result.mode, result.entry_guarantee, result.configuration_id);
        print_environment_limits();
        print_runtime_observations(&result);
        if let Some(error) = result.error {
            return Err(error);
        }
        Ok(())
    }
}

fn snapshot_command(store: &ConfigStore, args: &[String]) -> Result<(), String> {
    let container_id: Uuid = args
        .get(1)
        .ok_or("Container UUID required")?
        .parse()
        .map_err(|_| "invalid Container UUID")?;
    let snapshot = match args[0].as_str() {
        "prepare" => {
            let instance_id = match &args[2..] {
                [] => Uuid::new_v4(),
                [flag, value] if flag == "--instance" => {
                    value.parse().map_err(|_| "invalid Instance UUID")?
                }
                _ => return Err(
                    "usage: container prepare UUID [--instance UUID]; prepares configuration only"
                        .into(),
                ),
            };
            store
                .prepare_run_snapshot(container_id, instance_id)
                .map_err(|err| err.to_string())?
        }
        "show-snapshot" if args.len() == 3 => {
            let instance_id = args[2].parse().map_err(|_| "invalid Instance UUID")?;
            let snapshot = store
                .load_run_snapshot(container_id, instance_id)
                .map_err(|err| err.to_string())?;
            println!("{}", toml_snapshot(&snapshot)?);
            return Ok(());
        }
        _ => return Err("usage: container show-snapshot CONTAINER_UUID INSTANCE_UUID".into()),
    };
    println!("prepared_only\ttrue\nprocess_started\tfalse\ncontainer_id\t{}\ninstance_id\t{}\nsnapshot_id\t{}\nconfiguration_id\t{}\ncontent_digest\t{}\npath\t{}", snapshot.container_id, snapshot.instance_id, snapshot.snapshot_id, snapshot.configuration_id, snapshot.content_digest, store.run_snapshot_path(snapshot.container_id, snapshot.instance_id).display());
    Ok(())
}

fn toml_snapshot(snapshot: &envbox_core::RunSnapshot) -> Result<String, String> {
    // This explicit inspection command returns the complete user-requested configuration.
    toml::to_string_pretty(snapshot).map_err(|err| err.to_string())
}

#[cfg(windows)]
fn print_environment_limits() {
    println!("environment_scope\tprofile_information_view");
    println!("coverage_limit\tapplication_owned_dns_can_bypass_windows_dns_api_hooks");
    println!("coverage_limit\tun_injected_processes_and_sandboxed_browser_renderers_may_read_host_information");
    println!("entry_limit\tentry_gate_excludes_tls_callbacks_and_import_initializers");
    println!("observation_limit\thook_installation_does_not_verify_api_semantics_or_complete_process_tree_coverage");
}

#[cfg(windows)]
fn print_runtime_observations(result: &envbox_supervisor::RunResult) {
    print_runtime_facts(
        result.instance_id,
        result.root_pid,
        "root",
        result.environment_facts.as_ref(),
    );
    for member in &result.member_runtimes {
        print_runtime_facts(
            result.instance_id,
            member.pid,
            "observed_member",
            member.environment_facts.as_ref(),
        );
    }
}

#[cfg(windows)]
fn print_runtime_facts(
    instance_id: Uuid,
    pid: u32,
    subject: &str,
    facts: Option<&envbox_supervisor::EnvironmentRuntimeFacts>,
) {
    let (config_complete, profile_matches_snapshot) = facts.map_or_else(
        || ("unknown".into(), "unknown".into()),
        |facts| {
            (
                facts.config_complete.to_string(),
                facts.profile_matches_snapshot.to_string(),
            )
        },
    );
    println!(
        "runtime_observation\t{instance_id}\t{pid}\t{subject}\tconfig_complete={config_complete}\tprofile_matches_snapshot={profile_matches_snapshot}"
    );
    match facts {
        Some(facts) => {
            for hook in &facts.hooks {
                println!(
                    "installed_hook\t{instance_id}\t{pid}\t{}\t{}",
                    hook.group, hook.attached_api_count,
                );
            }
        }
        None => println!("installed_hooks\t{instance_id}\t{pid}\tunknown"),
    }
}
