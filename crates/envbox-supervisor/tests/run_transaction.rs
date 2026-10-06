#![cfg(windows)]
use envbox_core::*;
use envbox_storage::*;
use envbox_supervisor::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
#[ignore = "requires current Runtime and native console/no-TLS entry fixture; run from uninjected host"]
fn public_run_uses_immutable_snapshot_and_is_idempotent() {
    let dll = std::env::var("AURA_SUPERVISOR_RUN_DLL").expect("Runtime fixture required");
    let target = std::env::var("AURA_SUPERVISOR_RUN_TARGET").expect("native fixture required");
    std::env::set_var("ENVBOX_RUNTIME_DLL", dll);
    let root = std::env::temp_dir().join(format!("aura-supervisor-run-{}", Uuid::new_v4()));
    let store = ConfigStore::new(&root);
    let profile = EnvironmentProfile {
        id: Uuid::new_v4(),
        name: "snapshot fixture".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: DnsProfile::from_servers(DnsMode::VirtualView, vec!["1.1.1.1".parse().unwrap()]),
        environment: std::collections::HashMap::from([(
            "AURA_GATE_MARKER".into(),
            root.join("entry.txt").to_string_lossy().into(),
        )]),
        registry: RegistryProfile::default(),
        browser: Default::default(),
    };
    let container = Container::new("fixture", profile.id);
    let application = Application {
        id: Uuid::new_v4(),
        name: "native entry fixture".into(),
        launch: LaunchTarget::Executable {
            path: target.into(),
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
            applications: vec![application.clone()],
        })
        .unwrap();
    let instance_id = Uuid::new_v4();
    let snapshot = store
        .prepare_run_snapshot(container.id, instance_id)
        .unwrap();
    let mut changed = profile;
    changed.locale.locale_name = "fr-FR".into();
    changed.locale.ui_language = "fr-FR".into();
    changed.locale.region = "FR".into();
    store
        .save_profiles(&ProfileDocument {
            profiles: vec![changed],
        })
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let executable = std::env::current_exe().unwrap();
    let approved = ApprovedManager::from_file(&executable).unwrap();
    let server = std::thread::spawn(move || {
        serve(ServerConfig {
            store,
            approved_managers: vec![approved],
            managed_targets: Arc::default(),
            stop: server_stop,
            request_timeout: Duration::from_secs(2),
        })
    });
    let client = SupervisorClient {
        executable,
        timeout: Duration::from_secs(20),
    };
    let command = RunCommand {
        container_id: container.id,
        instance_id,
        application_id: application.id,
    };
    let mut request = Request {
        version: PROTOCOL_VERSION,
        generation: None,
        request_id: Uuid::new_v4().to_string(),
        command: "Run".into(),
        run: Some(command.clone()),
        container_id: None,
    };
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let ping = Request {
            command: "Ping".into(),
            run: None,
            container_id: None,
            ..request.clone()
        };
        if let Ok(hello) = client.request(ping) {
            request.generation = Some(hello.generation);
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    let first = client.request(request.clone()).unwrap();
    assert_eq!(first.status, "Running", "{:?}", first.run);
    let first_run = first.run.unwrap();
    assert!(first_run.root_pid > 0);
    let environment = first_run
        .environment_facts
        .as_ref()
        .expect("actual root observation");
    assert!(environment.config_complete && environment.profile_matches_snapshot);
    assert!(environment
        .hooks
        .iter()
        .any(|hook| hook.group == "dns" && hook.attached_api_count > 0));
    assert_eq!(
        first_run.member_runtimes[0].environment_facts.as_ref(),
        Some(environment)
    );
    assert!(!first_run.storage_policy_enforced);
    assert_eq!(first_run.configuration_id, snapshot.configuration_id);
    let replay = client.request(request.clone()).unwrap();
    assert_eq!(replay.run.unwrap().root_pid, first_run.root_pid);
    let conflict = Request {
        run: Some(RunCommand {
            application_id: Uuid::new_v4(),
            ..command.clone()
        }),
        ..request.clone()
    };
    assert_eq!(client.request(conflict).unwrap().status, "Failed");
    let query = Request {
        command: "RunStatus".into(),
        ..request.clone()
    };
    let finished = Instant::now() + Duration::from_secs(3);
    loop {
        if client.request(query.clone()).unwrap().status == "Exited" {
            break;
        }
        assert!(Instant::now() < finished);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read(root.join("entry.txt")).unwrap(), b"entry");
    let record = root
        .join("containers")
        .join(container.id.to_string())
        .join("runs")
        .join(format!("{instance_id}.json"));
    let saved: RunResult = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    assert_eq!(saved.root_pid, first_run.root_pid);
    stop.store(true, Ordering::Release);
    server.join().unwrap().unwrap();
    std::env::remove_var("ENVBOX_RUNTIME_DLL");
}
