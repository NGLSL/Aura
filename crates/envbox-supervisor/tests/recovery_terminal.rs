#![cfg(windows)]

use envbox_core::*;
use envbox_storage::*;
use envbox_supervisor::*;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use uuid::Uuid;

fn request(command: &str, generation: Option<String>, container: Option<Uuid>) -> Request {
    Request {
        version: PROTOCOL_VERSION,
        generation,
        request_id: Uuid::new_v4().to_string(),
        command: command.into(),
        run: None,
        container_id: container,
    }
}

fn process_absence_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
        || error.raw_os_error().is_some_and(|code| {
            // ManagedTarget::observe may expose HRESULT_FROM_WIN32(code).
            matches!((code as u32) & 0xffff, 2 | 3 | 6 | 87 | 1168)
        })
}

fn verified_absent_pid() -> u32 {
    // Probe candidates instead of assuming that a large PID is impossible.
    // If one is currently occupied, use another; an unexpected access error
    // fails the fixture rather than being treated as process absence.
    for candidate in [0x7fff_fff0, 0x7fff_ffef, 0x7fff_ffee, 0x7fff_ffed] {
        match ManagedTarget::observe(candidate) {
            Err(error) if process_absence_error(&error) => return candidate,
            Ok(_) => continue,
            Err(error) => panic!("cannot establish candidate PID {candidate} is absent: {error:?}"),
        }
    }
    panic!("all candidate PIDs are currently occupied; refusing an unsafe fixture")
}

#[test]
fn missing_job_with_absent_sealed_generations_stays_tracking_lost() {
    let root = std::env::temp_dir().join(format!("aura-no-job-terminal-{}", Uuid::new_v4()));
    let store = ConfigStore::new(&root);
    let profile = EnvironmentProfile {
        id: Uuid::new_v4(),
        name: "NoJob terminal fixture".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: DnsProfile::from_servers(
            DnsMode::VirtualView,
            vec!["1.1.1.1".parse().expect("fixture DNS address")],
        ),
        environment: Default::default(),
        registry: Default::default(),
        browser: Default::default(),
    };
    let container = Container::new("NoJob terminal", profile.id);
    let instance_id = Uuid::new_v4();
    let application_id = Uuid::new_v4();
    let application = Application {
        id: application_id,
        name: "terminal fixture".into(),
        launch: LaunchTarget::Executable {
            path: std::env::current_exe().expect("test executable"),
        },
        working_directory: None,
        arguments: vec![],
        default_profile_id: profile.id,
        inherit_children: true,
        console_host: ConsoleHost::Direct,
        audit: false,
    };
    store
        .save_profiles(&ProfileDocument {
            profiles: vec![profile.clone()],
        })
        .unwrap();
    store
        .save_containers(&ContainerDocument {
            containers: vec![container.clone()],
            ..Default::default()
        })
        .unwrap();
    store
        .save_applications(&ApplicationDocument {
            applications: vec![application],
        })
        .unwrap();
    let snapshot = store
        .prepare_run_snapshot(container.id, instance_id)
        .unwrap();
    let expected = envbox_launcher::ipc::profile_to_message_with_flags(
        &profile,
        &instance_id.to_string(),
        true,
        false,
    );
    let expected_config_sha256 = format!("{:x}", Sha256::digest(expected.encode_line().as_bytes()));
    // The fixture exercises the real OpenProcess generation check rather than
    // assuming a PID range is unused.
    let absent_pid = verified_absent_pid();
    let absent_generation = 1;
    let missing_job = format!("Local\\AuraRun-{}-{}", instance_id, Uuid::new_v4());
    let result = RunResult {
        record_schema: 3,
        request_id: "old-request".into(),
        supervisor_generation: "old-supervisor".into(),
        job_name: missing_job,
        snapshot_digest: snapshot.content_digest.clone(),
        container_id: container.id,
        instance_id,
        application_id,
        profile_id: profile.id,
        runtime_module_path: std::env::current_exe().unwrap(),
        runtime_module_sha256: "fixture-sha".into(),
        runtime_config_sha256: expected_config_sha256.clone(),
        runtime_version: env!("CARGO_PKG_VERSION").into(),
        environment_facts: None,
        audit: false,
        inherit_children: true,
        known_members: vec![ProcessIdentity {
            pid: absent_pid,
            creation_time: absent_generation,
        }],
        member_runtimes: vec![MemberRuntimeIdentity {
            pid: absent_pid,
            creation_time: absent_generation,
            module_path: std::env::current_exe().unwrap(),
            module_sha256: "fixture-sha".into(),
            config_sha256: expected_config_sha256,
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            environment_facts: None,
        }],
        root_pid: absent_pid,
        creation_time: absent_generation,
        mode: "compatibility".into(),
        entry_guarantee: "verified_pe_entry_no_tls".into(),
        storage_policy_enforced: false,
        configuration_id: snapshot.configuration_id.clone(),
        error: None,
        state: "Running".into(),
    };
    let record_path = root
        .join("containers")
        .join(container.id.to_string())
        .join("runs")
        .join(format!("{instance_id}.json"));
    std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    std::fs::write(&record_path, serde_json::to_vec(&result).unwrap()).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let executable = std::env::current_exe().unwrap();
    let server = std::thread::spawn(move || {
        serve(ServerConfig {
            store,
            approved_managers: vec![ApprovedManager::from_file(&executable).unwrap()],
            managed_targets: Arc::default(),
            stop: server_stop,
            request_timeout: Duration::from_secs(2),
        })
        .unwrap();
    });
    let client = SupervisorClient {
        executable: std::env::current_exe().unwrap(),
        timeout: Duration::from_secs(5),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let hello = loop {
        if let Ok(response) = client.request(request("Ping", None, None)) {
            break response;
        }
        if server.is_finished() {
            panic!(
                "Supervisor exited before accepting Ping: {:?}",
                server.join()
            );
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Supervisor did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let listed = client
        .request(request("List", Some(hello.generation), Some(container.id)))
        .unwrap();
    assert_eq!(listed.status, "Ok", "{listed:?}");
    assert_eq!(listed.instances.len(), 1);
    let recovered = &listed.instances[0].result;
    assert_eq!(recovered.state, "TrackingLost", "{recovered:?}");
    assert!(recovered
        .error
        .as_deref()
        .is_some_and(|error| error.contains("complete process tree cannot be proven")));

    stop.store(true, Ordering::Release);
    server.join().unwrap();
    let persisted: RunResult =
        serde_json::from_slice(&std::fs::read(record_path).unwrap()).unwrap();
    assert_eq!(persisted.state, "TrackingLost");
    assert!(persisted
        .error
        .unwrap()
        .contains("tracking Job no longer exists"));
    std::fs::remove_dir_all(root).unwrap();
}
