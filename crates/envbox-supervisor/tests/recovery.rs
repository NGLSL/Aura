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

fn member_image(pid: u32) -> String {
    use windows::Win32::{Foundation::CloseHandle, System::Threading::*};
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return "not queryable".into();
        };
        let mut path = [0u16; 32768];
        let mut length = path.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(handle);
        if result.is_err() {
            "image unavailable".into()
        } else {
            String::from_utf16_lossy(&path[..length as usize])
        }
    }
}

#[test]
#[ignore = "requires latest escrow/reconnect Runtime and uninjected host"]
fn supervisor_crash_reconfirms_a_b_and_preserves_host() {
    exercise_crash(false, false, false, false, false);
}

#[test]
#[ignore = "requires current escrow Runtime, native child fixture and uninjected host"]
fn supervisor_crash_recovers_child_after_root_exit() {
    exercise_crash(true, false, false, false, false);
}

#[test]
#[ignore = "requires frozen Runtime pair and both native child architectures"]
fn mixed_recovery_x64_root_x86_child() {
    exercise_crash(true, false, true, false, false);
}

#[test]
#[ignore = "requires frozen Runtime pair and both native child architectures"]
fn mixed_recovery_x86_root_x64_child() {
    exercise_crash(true, true, false, false, false);
}

#[test]
#[ignore = "requires frozen Runtime pair and native mixed parent fixture"]
fn mixed_recovery_x64_root_alive_x86_child() {
    exercise_crash(true, false, true, true, false);
}

#[test]
#[ignore = "requires frozen Runtime pair and native mixed parent fixture"]
fn mixed_recovery_x86_root_alive_x64_child() {
    exercise_crash(true, true, false, true, false);
}

#[test]
#[ignore = "requires current Runtime and immutable schema-two fixture journal"]
fn legacy_recovery_schema_two_same_architecture() {
    exercise_crash(false, false, false, false, true);
}

#[test]
#[ignore = "requires current Runtime and immutable schema-two fixture journal"]
fn legacy_recovery_schema_two_child_after_root_exit() {
    exercise_crash(true, false, false, false, true);
}

#[test]
#[ignore = "requires current pair; old mixed journal has no member module evidence"]
fn legacy_recovery_schema_two_mixed_stays_lost() {
    exercise_crash(true, false, true, false, true);
}

fn exercise_crash(child_tree: bool, root32: bool, child32: bool, live_root: bool, legacy: bool) {
    let dll = std::env::var("AURA_SUPERVISOR_RUN_DLL").expect("current escrow bundle required");
    let target64 =
        std::env::var("AURA_SUPERVISOR_RUN_TARGET").expect("native wait fixture required");
    let target = if child32 {
        target64.replace("gate-fixture64", "gate-fixture32")
    } else {
        target64
    };
    let mixed_tree = child_tree && root32 != child32;
    let target = if mixed_tree {
        target.replace(
            "envbox-startup-gate-entry.exe",
            "envbox-startup-gate-gui.exe",
        )
    } else {
        target
    };
    std::env::set_var(
        "ENVBOX_RUNTIME_DLL",
        if root32 {
            dll.replace("envbox-runtime64.dll", "envbox-runtime32.dll")
        } else {
            dll
        },
    );
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
            ("AURA_RECOVERY_CHILD_TARGET".into(), target.clone()),
            (
                "AURA_RECOVERY_PARENT_EXIT".into(),
                if live_root { "0" } else { "1" }.into(),
            ),
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
            path: if mixed_tree {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target")
                    .join(if root32 {
                        "mixed-recovery-fixture32"
                    } else {
                        "mixed-recovery-fixture64"
                    })
                    .join("Release/envbox-recovery-parent.exe")
            } else if child_tree {
                if root32 {
                    std::path::PathBuf::from(std::env::var("SystemRoot").unwrap())
                        .join("SysWOW64/cmd.exe")
                } else {
                    std::env::var("ComSpec").unwrap().into()
                }
            } else {
                target.clone().into()
            },
        },
        working_directory: None,
        arguments: if child_tree && !mixed_tree {
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
                if view.result.state == "Running"
                    && view.result.known_members.len() == view.process_ids.len()
                    && view.result.member_runtimes.len() == view.process_ids.len()
                    && ((!live_root
                        && !view.process_ids.is_empty()
                        && !view.process_ids.contains(&result.root_pid))
                        || (live_root
                            && view.process_ids.len() == 2
                            && view.process_ids.contains(&result.root_pid)))
                {
                    assert_eq!(view.result.known_members.len(), view.process_ids.len());
                    assert_eq!(view.result.member_runtimes.len(), view.process_ids.len());
                    assert_eq!(view.result.state, "Running", "{:?}", view.result.error);
                    assert!(view.result.member_runtimes.iter().any(|proof| proof
                        .module_path
                        .file_name()
                        .unwrap()
                        == if child32 {
                            "envbox-runtime32.dll"
                        } else {
                            "envbox-runtime64.dll"
                        }));
                    println!(
                        "root_live={live_root} pid={} sealed_members={:?} modules={:?}",
                        result.root_pid,
                        view.process_ids,
                        view.result
                            .member_runtimes
                            .iter()
                            .map(|proof| &proof.module_path)
                            .collect::<Vec<_>>()
                    );
                    break;
                }
                if Instant::now() >= deadline {
                    for pid in &view.process_ids {
                        eprintln!("actual_job_member pid={pid} image={}", member_image(*pid));
                    }
                    eprintln!(
                        "child_entry_marker={} parent_fixture={}",
                        root.join("entry-a.txt").exists(),
                        mixed_tree
                    );
                }
                assert!(
                    Instant::now() < deadline,
                    "native parent/child did not reach verified membership: {:?}",
                    view
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        } else {
            assert!(job.stats().unwrap().process_ids.contains(&result.root_pid));
        }
    }
    if legacy {
        for command in [&command_a, &command_b] {
            if mixed_tree && command.container_id == b.id {
                continue;
            }
            let path = root
                .join("containers")
                .join(command.container_id.to_string())
                .join("runs")
                .join(format!("{}.json", command.instance_id));
            let mut journal: RunResult =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            journal.record_schema = 2;
            journal.member_runtimes.clear();
            std::fs::write(path, serde_json::to_vec(&journal).unwrap()).unwrap();
        }
    }
    // Kill only our owned test-server child. No PID lookup or product service is used.
    server.kill().unwrap();
    server.wait().unwrap();
    assert!(host.try_wait().unwrap().is_none());
    let mut restarted = launch(&root);
    let mut next = hello(&client);
    assert_eq!(next.supervisor_pid, restarted.id());
    assert_ne!(first.generation, next.generation);
    assert!(client
        .request(request(Some(first.generation), "StopAll", None, Some(a.id)))
        .is_err());
    if legacy && mixed_tree {
        let lost = client
            .request(request(
                Some(next.generation.clone()),
                "RunStatus",
                Some(command_a.clone()),
                None,
            ))
            .unwrap();
        assert_eq!(lost.status, "TrackingLost");
        let lost_a = lost.run.unwrap();
        assert!(
            lost_a
                .error
                .as_ref()
                .unwrap()
                .contains("sealed Runtime is not loaded"),
            "{:?}",
            lost_a.error
        );
        let stop_a = client
            .request(request(
                Some(next.generation.clone()),
                "Stop",
                Some(command_a.clone()),
                Some(a.id),
            ))
            .unwrap();
        assert_eq!(stop_a.status, "NotControlled");
        let a_job = envbox_launcher::InstanceJob::open_named(&lost_a.job_name).unwrap();
        a_job.verify_tracking_limits().unwrap();
        assert!(
            a_job.stats().unwrap().active_processes > 0,
            "failed old-bundle recovery must leave target alive"
        );
        let b_view = client
            .request(request(
                Some(next.generation.clone()),
                "RunStatus",
                Some(command_b.clone()),
                None,
            ))
            .unwrap();
        assert_eq!(b_view.status, "Running", "{:?}", b_view.run);
        assert!(host.try_wait().unwrap().is_none());
        client
            .request(request(Some(next.generation), "StopAll", None, Some(b.id)))
            .unwrap();
        // Only this trusted fixture's original A Job is explicitly cleaned.
        // The public client was denied control; it did not claim these PIDs.
        a_job.terminate().unwrap();
        host.kill().unwrap();
        host.wait().unwrap();
        restarted.kill().unwrap();
        restarted.wait().unwrap();
        println!("legacy_schema2 mixed_no_member_evidence=TrackingLost publicStop=NotControlled target_alive_after_refusal=true B_recovered=true host_alive=true");
        return;
    }
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
    // A second real crash consumes the newly sealed record, rather than the
    // original root's proof. This catches stale known_members after root exit.
    restarted.kill().unwrap();
    restarted.wait().unwrap();
    restarted = launch(&root);
    let second = hello(&client);
    assert_ne!(next.generation, second.generation);
    next = second;
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
        let result = response.run.unwrap();
        assert_eq!(result.member_runtimes.len(), result.known_members.len());
        assert_eq!(result.supervisor_generation, next.generation);
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
    println!("supervisor_crash child_tree={child_tree} root32={root32} child32={child32} fresh_reconfirmed=A,B stale_generation_rejected=true stopA_keepsB_and_host=true corruptA_visibleLost_B_newRun=true");
}
