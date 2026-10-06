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

fn profile(marker: &std::path::Path) -> EnvironmentProfile {
    EnvironmentProfile {
        id: Uuid::new_v4(),
        name: "scope fixture".into(),
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
            ("AURA_GATE_MARKER".into(), marker.to_string_lossy().into()),
            ("AURA_GATE_WAIT_MS".into(), "30000".into()),
        ]),
        registry: Default::default(),
        browser: Default::default(),
        identity: Default::default(),
    }
}
fn message(command: &str, generation: &str, scope: Uuid, run: Option<RunCommand>) -> Request {
    Request {
        version: PROTOCOL_VERSION,
        generation: Some(generation.into()),
        request_id: Uuid::new_v4().to_string(),
        command: command.into(),
        run,
        container_id: Some(scope),
    }
}
struct Host(std::process::Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires frozen Runtime/native wait fixture and an uninjected host"]
fn stop_all_preserves_b_and_same_executable_host_and_serializes_run() {
    use std::os::windows::process::CommandExt;
    let dll = std::env::var("AURA_SUPERVISOR_RUN_DLL").expect("Runtime required");
    let target = std::env::var("AURA_SUPERVISOR_RUN_TARGET").expect("native wait fixture required");
    std::env::set_var("ENVBOX_RUNTIME_DLL", dll);
    let root = std::env::temp_dir().join(format!("aura-stop-scope-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let store = ConfigStore::new(&root);
    let mut a_profile = profile(&root.join("a-entry.txt"));
    let b_profile = profile(&root.join("b-entry.txt"));
    let a = Container::new("A", a_profile.id);
    let b = Container::new("B", b_profile.id);
    let app = Application {
        id: Uuid::new_v4(),
        name: "same executable".into(),
        launch: LaunchTarget::Executable {
            path: target.clone().into(),
        },
        working_directory: None,
        arguments: vec![],
        default_profile_id: a_profile.id,
        inherit_children: true,
        console_host: ConsoleHost::Direct,
        audit: false,
    };
    store
        .save_profiles(&ProfileDocument {
            profiles: vec![a_profile.clone(), b_profile.clone()],
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
    let a_command = RunCommand {
        container_id: a.id,
        instance_id: Uuid::new_v4(),
        application_id: app.id,
    };
    let b_command = RunCommand {
        container_id: b.id,
        instance_id: Uuid::new_v4(),
        application_id: app.id,
    };
    store
        .prepare_run_snapshot(a.id, a_command.instance_id)
        .unwrap();
    store
        .prepare_run_snapshot(b.id, b_command.instance_id)
        .unwrap();
    let mut host = Host(
        std::process::Command::new(&target)
            .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
            .env("AURA_GATE_MARKER", root.join("host-entry.txt"))
            .env("AURA_GATE_WAIT_MS", "30000")
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !root.join("host-entry.txt").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !runtime_loaded(host.0.id()),
        "same-exe host control must be uninjected"
    );
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let executable = std::env::current_exe().unwrap();
    let approved = ApprovedManager::from_file(&executable).unwrap();
    let server_store = store.clone();
    let server = std::thread::spawn(move || {
        serve(ServerConfig {
            store: server_store,
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
    let deadline = Instant::now() + Duration::from_secs(3);
    let hello = loop {
        if let Ok(response) = client.request(Request {
            generation: None,
            ..message("Ping", "", a.id, None)
        }) {
            break response;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    for run in [&a_command, &b_command] {
        let response = client
            .request(message(
                "Run",
                &hello.generation,
                run.container_id,
                Some(run.clone()),
            ))
            .unwrap();
        assert_eq!(response.status, "Running", "{:?}", response.run);
        assert!(runtime_loaded(response.run.unwrap().root_pid));
    }
    let reconnected = client.clone();
    let a_list = reconnected
        .request(message("List", &hello.generation, a.id, None))
        .unwrap();
    assert_eq!(a_list.instances.len(), 1);
    assert_eq!(a_list.instances[0].result.state, "Running");
    let stop_a = message("StopAll", &hello.generation, a.id, None);
    let stopped = reconnected.request(stop_a.clone()).unwrap();
    assert_eq!(stopped.status, "Ok");
    assert_eq!(stopped.instances.len(), 1);
    assert_eq!(stopped.instances[0].result.state, "Stopped");
    assert_eq!(
        client
            .request(message(
                "RunStatus",
                &hello.generation,
                b.id,
                Some(b_command.clone())
            ))
            .unwrap()
            .status,
        "Running"
    );
    assert!(host.0.try_wait().unwrap().is_none());
    let a2 = RunCommand {
        instance_id: Uuid::new_v4(),
        ..a_command.clone()
    };
    a_profile.environment.insert(
        "AURA_GATE_MARKER".into(),
        root.join("a2-entry.txt").to_string_lossy().into(),
    );
    store
        .save_profiles(&ProfileDocument {
            profiles: vec![a_profile.clone(), b_profile.clone()],
        })
        .unwrap();
    store.prepare_run_snapshot(a.id, a2.instance_id).unwrap();
    assert_eq!(
        client
            .request(message("Run", &hello.generation, a.id, Some(a2.clone())))
            .unwrap()
            .status,
        "Running"
    );
    let repeated = client.request(stop_a).unwrap();
    assert_eq!(repeated.instances.len(), 1);
    assert_eq!(
        repeated.instances[0].result.instance_id,
        a_command.instance_id
    );
    assert_eq!(
        client
            .request(message(
                "RunStatus",
                &hello.generation,
                a.id,
                Some(a2.clone())
            ))
            .unwrap()
            .status,
        "Running",
        "StopAll replay must preserve later Runs"
    );
    let stop_one = message("Stop", &hello.generation, a.id, Some(a2));
    assert_eq!(
        client.request(stop_one.clone()).unwrap().instances[0]
            .result
            .state,
        "Stopped"
    );
    assert_eq!(
        client.request(stop_one).unwrap().instances[0].result.state,
        "Stopped"
    );
    // Simultaneous connections are linearized by the single command owner.
    let a3 = RunCommand {
        instance_id: Uuid::new_v4(),
        ..a_command
    };
    a_profile.environment.insert(
        "AURA_GATE_MARKER".into(),
        root.join("a3-entry.txt").to_string_lossy().into(),
    );
    store
        .save_profiles(&ProfileDocument {
            profiles: vec![a_profile, b_profile],
        })
        .unwrap();
    store.prepare_run_snapshot(a.id, a3.instance_id).unwrap();
    let run_message = message("Run", &hello.generation, a.id, Some(a3.clone()));
    let stop_message = message("StopAll", &hello.generation, a.id, None);
    let (run_response, stop_response) = std::thread::scope(|scope| {
        let run = scope.spawn(|| client.request(run_message).unwrap());
        let stop = scope.spawn(|| reconnected.request(stop_message).unwrap());
        (run.join().unwrap(), stop.join().unwrap())
    });
    assert_eq!(run_response.status, "Running");
    let included = stop_response
        .instances
        .iter()
        .any(|view| view.result.instance_id == a3.instance_id);
    let final_state = client
        .request(message(
            "RunStatus",
            &hello.generation,
            a.id,
            Some(a3.clone()),
        ))
        .unwrap()
        .status;
    assert_eq!(final_state, if included { "Stopped" } else { "Running" });
    println!("A/B/host same-exe scope passed; concurrent A3 included={included}, state={final_state}; host_pid={}", host.0.id());
    assert_eq!(
        client
            .request(message(
                "RunStatus",
                &hello.generation,
                b.id,
                Some(b_command.clone())
            ))
            .unwrap()
            .status,
        "Running"
    );
    assert!(host.0.try_wait().unwrap().is_none());
    for command in [a3, b_command] {
        assert_eq!(
            client
                .request(message(
                    "Stop",
                    &hello.generation,
                    command.container_id,
                    Some(command)
                ))
                .unwrap()
                .status,
            "Ok"
        );
    }
    stop.store(true, Ordering::Release);
    server.join().unwrap().unwrap();
    std::env::remove_var("ENVBOX_RUNTIME_DLL");
}

fn runtime_loaded(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES};
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    unsafe {
        let snapshot =
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid).unwrap();
        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        Module32FirstW(snapshot, &mut entry).unwrap();
        loop {
            let length = entry
                .szModule
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(entry.szModule.len());
            let name = String::from_utf16_lossy(&entry.szModule[..length]).to_ascii_lowercase();
            if name.starts_with("envbox-runtime") {
                CloseHandle(snapshot).unwrap();
                return true;
            }
            if Module32NextW(snapshot, &mut entry).is_err() {
                assert_eq!(GetLastError(), ERROR_NO_MORE_FILES);
                CloseHandle(snapshot).unwrap();
                return false;
            }
        }
    }
}
