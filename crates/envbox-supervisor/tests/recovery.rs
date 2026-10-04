#![cfg(windows)]
use envbox_core::*;
use envbox_storage::*;
use envbox_supervisor::*;
use std::os::windows::process::CommandExt;
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[test]
#[ignore = "explicit isolated fixture server; never a production manager allowlist"]
fn recovery_server_fixture() {
    let Some(root) = std::env::var_os("AURA_RECOVERY_FIXTURE_STORE") else {
        return;
    };
    serve(ServerConfig {
        store: ConfigStore::new(root),
        approved_managers: vec![
            ApprovedManager::from_file(&std::env::current_exe().unwrap()).unwrap(),
        ],
        managed_targets: Arc::default(),
        stop: Arc::new(AtomicBool::new(false)),
        request_timeout: Duration::from_secs(2),
    })
    .unwrap();
}

fn request(
    generation: Option<String>,
    command: &str,
    target: Option<RunCommand>,
    container: Option<Uuid>,
) -> Request {
    Request {
        version: PROTOCOL_VERSION,
        generation,
        request_id: Uuid::new_v4().to_string(),
        command: command.into(),
        run: target,
        container_id: container,
    }
}
struct FixtureChild(std::process::Child);
impl std::ops::Deref for FixtureChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for FixtureChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch(root: &std::path::Path) -> FixtureChild {
    FixtureChild(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "recovery_server_fixture",
                "--nocapture",
            ])
            .env("AURA_RECOVERY_FIXTURE_STORE", root)
            .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap(),
    )
}
fn hello(client: &SupervisorClient) -> Response {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(response) = client.request(request(None, "Ping", None, None)) {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "fixture server did not become available"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires latest escrow/reconnect Runtime and uninjected host"]
fn supervisor_crash_reconfirms_a_b_and_preserves_host() {
    exercise_crash(false);
}

#[test]
#[ignore = "requires current escrow Runtime, native child fixture and uninjected host"]
fn supervisor_crash_recovers_child_after_root_exit() {
    exercise_crash(true);
}

fn exercise_crash(child_tree: bool) {
    let dll = std::env::var("AURA_SUPERVISOR_RUN_DLL").expect("current escrow bundle required");
    let target = std::env::var("AURA_SUPERVISOR_RUN_TARGET").expect("native wait fixture required");
    std::env::set_var("ENVBOX_RUNTIME_DLL", dll);
    let root = std::env::temp_dir().join(format!("aura-recovery-{}", Uuid::new_v4()));
    let store = ConfigStore::new(&root);
    let profile = EnvironmentProfile {
        id: Uuid::new_v4(),
        name: "recovery fixture".into(),
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
        environment: std::collections::HashMap::from([
            ("AURA_GATE_WAIT_MS".into(), "30000".into()),
            (
                "AURA_GATE_MARKER".into(),
                root.join("entry-a.txt").to_string_lossy().into(),
            ),
        ]),
        registry: Default::default(),
        browser: Default::default(),
    };
    let a = Container::new("A", profile.id);
    let b = Container::new("B", profile.id);
    let app = Application {
        id: Uuid::new_v4(),
        name: "wait".into(),
        launch: LaunchTarget::Executable {
            path: if child_tree {
                std::env::var("ComSpec").unwrap().into()
            } else {
                target.clone().into()
            },
        },
        working_directory: None,
        arguments: if child_tree {
            vec![
                "/d".into(),
                "/c".into(),
                "start".into(),
                "".into(),
                "/b".into(),
                target.clone(),
            ]
        } else {
            vec![]
        },
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
            containers: vec![a.clone(), b.clone()],
            ..Default::default()
        })
        .unwrap();
    store
        .save_applications(&ApplicationDocument {
            applications: vec![app.clone()],
        })
        .unwrap();
    let command_a = RunCommand {
        container_id: a.id,
        instance_id: Uuid::new_v4(),
        application_id: app.id,
    };
    let command_b = RunCommand {
        container_id: b.id,
        instance_id: Uuid::new_v4(),
        application_id: app.id,
    };
    store
        .prepare_run_snapshot(a.id, command_a.instance_id)
        .unwrap();
    let mut profile_b = profile;
    profile_b.environment.insert(
        "AURA_GATE_MARKER".into(),
        root.join("entry-b.txt").to_string_lossy().into(),
    );
    store
        .save_profiles(&ProfileDocument {
            profiles: vec![profile_b],
        })
        .unwrap();
    store
        .prepare_run_snapshot(b.id, command_b.instance_id)
        .unwrap();
    let mut host = FixtureChild(
        std::process::Command::new(&target)
            .env("AURA_GATE_WAIT_MS", "30000")
            .env("AURA_GATE_MARKER", root.join("host.txt"))
            .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap(),
    );
    let client = SupervisorClient {
        executable: std::env::current_exe().unwrap(),
        timeout: Duration::from_secs(20),
    };
    let mut server = launch(&root);
    let first = hello(&client);
    assert_eq!(first.supervisor_pid, server.id());
    for command in [&command_a, &command_b] {
        let response = client
            .request(request(
                Some(first.generation.clone()),
                "Run",
                Some(command.clone()),
                None,
            ))
            .unwrap();
        assert_eq!(response.status, "Running", "{:?}", response.run);
        let result = response.run.unwrap();
        let job = envbox_launcher::InstanceJob::open_named(&result.job_name).unwrap();
        if child_tree {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let listed = client
                    .request(request(
                        Some(first.generation.clone()),
                        "List",
                        None,
                        Some(command.container_id),
                    ))
                    .unwrap();
                let view = listed
                    .instances
                    .iter()
                    .find(|view| view.result.instance_id == command.instance_id)
                    .unwrap();
                if !view.process_ids.is_empty() && !view.process_ids.contains(&result.root_pid) {
                    assert_eq!(view.result.known_members.len(), view.process_ids.len());
                    println!(
                        "root_exited pid={} surviving_sealed_children={:?}",
                        result.root_pid, view.process_ids
                    );
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "native child did not survive CMD root exit"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        } else {
            assert!(job.stats().unwrap().process_ids.contains(&result.root_pid));
        }
    }
    // Kill only our owned test-server child. No PID lookup or product service is used.
    server.kill().unwrap();
    server.wait().unwrap();
    assert!(host.try_wait().unwrap().is_none());
    let mut restarted = launch(&root);
    let next = hello(&client);
    assert_eq!(next.supervisor_pid, restarted.id());
    assert_ne!(first.generation, next.generation);
    assert!(client
        .request(request(Some(first.generation), "StopAll", None, Some(a.id)))
        .is_err());
    for command in [&command_a, &command_b] {
        let response = client
            .request(request(
                Some(next.generation.clone()),
                "RunStatus",
                Some(command.clone()),
                None,
            ))
            .unwrap();
        assert_eq!(response.status, "Running", "{:?}", response.run);
        assert_eq!(response.run.unwrap().supervisor_generation, next.generation);
    }
    let stopped = client
        .request(request(
            Some(next.generation.clone()),
            "StopAll",
            None,
            Some(a.id),
        ))
        .unwrap();
    assert!(matches!(stopped.status.as_str(), "Ok" | "Partial"));
    let b_view = client
        .request(request(
            Some(next.generation.clone()),
            "RunStatus",
            Some(command_b.clone()),
            None,
        ))
        .unwrap();
    assert_eq!(b_view.status, "Running");
    assert!(host.try_wait().unwrap().is_none());
    // Damage only A's fixture journal while B still owns a live Job. A's
    // malformed record must remain visible without denying B's new Run.
    restarted.kill().unwrap();
    restarted.wait().unwrap();
    let a_record = root
        .join("containers")
        .join(a.id.to_string())
        .join("runs")
        .join(format!("{}.json", command_a.instance_id));
    std::fs::write(a_record, "{").unwrap();
    let mut third_server = launch(&root);
    let third = hello(&client);
    let lost = client
        .request(request(
            Some(third.generation.clone()),
            "List",
            None,
            Some(a.id),
        ))
        .unwrap();
    assert_eq!(lost.instances.len(), 1);
    assert_eq!(lost.instances[0].result.state, "TrackingLost");
    assert!(lost.instances[0]
        .result
        .error
        .as_ref()
        .unwrap()
        .contains("invalid Run record"));
    let new_a = RunCommand {
        instance_id: Uuid::new_v4(),
        ..command_a.clone()
    };
    store.prepare_run_snapshot(a.id, new_a.instance_id).unwrap();
    let denied_a = client
        .request(request(
            Some(third.generation.clone()),
            "Run",
            Some(new_a),
            None,
        ))
        .unwrap();
    assert_eq!(denied_a.status, "Failed");
    assert!(denied_a
        .run
        .unwrap()
        .error
        .unwrap()
        .contains("TrackingLost"));
    let mut document = store.load_profiles().unwrap();
    document.profiles[0].environment.insert(
        "AURA_GATE_MARKER".into(),
        root.join("entry-b2.txt").to_string_lossy().into(),
    );
    store.save_profiles(&document).unwrap();
    let new_b = RunCommand {
        instance_id: Uuid::new_v4(),
        ..command_b.clone()
    };
    store.prepare_run_snapshot(b.id, new_b.instance_id).unwrap();
    let started_b = client
        .request(request(
            Some(third.generation.clone()),
            "Run",
            Some(new_b),
            None,
        ))
        .unwrap();
    assert_eq!(started_b.status, "Running", "{:?}", started_b.run);
    client
        .request(request(Some(third.generation), "StopAll", None, Some(b.id)))
        .unwrap();
    host.kill().unwrap();
    host.wait().unwrap();
    third_server.kill().unwrap();
    third_server.wait().unwrap();
    println!("supervisor_crash child_tree={child_tree} fresh_reconfirmed=A,B stale_generation_rejected=true stopA_keepsB_and_host=true corruptA_visibleLost_B_newRun=true");
}
