#![cfg(windows)]
use envbox_supervisor::*;
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

// The production endpoint is unique for SID + exact integrity level. These
// two fixtures intentionally install different approved server artifacts at
// that same endpoint, so only their ownership must be serialized. The hidden
// startup fixture still races four separate manager processes internally.
static PRINCIPAL_ENDPOINT: Mutex<()> = Mutex::new(());

fn request(command: &str, generation: Option<String>) -> Request {
    Request {
        version: PROTOCOL_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        command: command.into(),
        generation,
        run: None,
        container_id: None,
    }
}

#[test]
fn authenticated_channel_and_negative_cases() {
    let _endpoint = PRINCIPAL_ENDPOINT.lock().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let targets = Arc::new(Mutex::new(HashSet::new()));
    let executable = std::env::current_exe().unwrap();
    let server_stop = stop.clone();
    let server_targets = targets.clone();
    let approved = ApprovedManager::from_file(&executable).unwrap();
    let server = std::thread::spawn(move || {
        serve(ServerConfig {
            store: envbox_storage::ConfigStore::new(
                std::env::temp_dir().join("aura-supervisor-unused-store"),
            ),
            approved_managers: vec![approved],
            managed_targets: server_targets,
            stop: server_stop,
            request_timeout: Duration::from_millis(250),
        })
    });
    let client = SupervisorClient {
        executable,
        timeout: Duration::from_secs(2),
    };
    let end = Instant::now() + Duration::from_secs(2);
    let first = loop {
        match client.request(request("Ping", None)) {
            Ok(response) => break response,
            Err(error) => {
                assert!(Instant::now() < end, "{error}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    assert_eq!(first.status, "Ok");
    let mut list = request("List", Some(first.generation.clone()));
    list.container_id = Some(uuid::Uuid::new_v4());
    assert_eq!(client.request(list.clone()).unwrap().status, "NotOwned");
    let mut stop_all = Request {
        command: "StopAll".into(),
        ..list.clone()
    };
    let empty = client.request(stop_all.clone()).unwrap();
    assert_eq!(empty.status, "NotOwned");
    assert!(empty.instances.is_empty());
    assert_eq!(client.request(stop_all.clone()).unwrap().status, "NotOwned");
    stop_all.container_id = Some(uuid::Uuid::new_v4());
    assert_eq!(client.request(stop_all).unwrap().status, "NotOwned");
    let unknown = Request {
        command: "Stop".into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        run: Some(RunCommand {
            container_id: list.container_id.unwrap(),
            instance_id: uuid::Uuid::new_v4(),
            application_id: uuid::Uuid::new_v4(),
        }),
        ..list
    };
    assert_eq!(client.request(unknown).unwrap().status, "NotOwned");
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        client
            .request_cancellable(request("Ping", None), Some(&cancelled))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(
        client
            .request(request("Reset", Some(first.generation.clone())))
            .unwrap()
            .status,
        "Unsupported"
    );
    assert_eq!(
        client
            .request(request("Ping", Some("stale".into())))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    let mut wrong_version = request("Ping", None);
    wrong_version.version = 999;
    assert_eq!(
        client.request(wrong_version).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    targets
        .lock()
        .unwrap()
        .insert(ManagedTarget::observe(std::process::id()).unwrap());
    assert_eq!(
        client.request(request("Delete", None)).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    targets.lock().unwrap().clear();
    let after = client.request(request("Ping", None)).unwrap();
    assert_eq!(
        first.generation, after.generation,
        "disconnect must preserve server"
    );
    // A connected client sending no bytes expires, allowing subsequent managers.
    unsafe {
        use windows::Win32::Foundation::*;
        use windows::Win32::Storage::FileSystem::*;
        let endpoint: Vec<u16> = client
            .endpoint()
            .unwrap()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        std::thread::sleep(Duration::from_millis(30));
        let idle = CreateFileW(
            windows::core::PCWSTR(endpoint.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .unwrap();
        let begin = Instant::now();
        let recovered = client.request(request("Ping", None)).unwrap();
        assert_eq!(recovered.status, "Ok");
        assert!(
            begin.elapsed() < Duration::from_secs(1),
            "idle pipe exceeded request deadline"
        );
        CloseHandle(idle).unwrap();
    }
    stop.store(true, Ordering::Release);
    server.join().unwrap().unwrap();

    // An explicit empty fixture allowlist cannot be widened by a wire request.
    let denied_stop = Arc::new(AtomicBool::new(false));
    let server_stop = denied_stop.clone();
    let denied_server = std::thread::spawn(move || {
        serve(ServerConfig {
            store: envbox_storage::ConfigStore::new(
                std::env::temp_dir().join("aura-supervisor-unused-store"),
            ),
            approved_managers: vec![],
            managed_targets: Arc::default(),
            stop: server_stop,
            request_timeout: Duration::from_millis(250),
        })
    });
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(
        client.request(request("Ping", None)).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    denied_stop.store(true, Ordering::Release);
    denied_server.join().unwrap().unwrap();
}

#[test]
#[ignore = "invoked only by the isolated management artifact fixture"]
fn manager_fixture_helper() {
    let fixture = std::env::var_os("AURA_SUPERVISOR_TEST_FIXTURE").expect("fixture required");
    let output = std::env::var_os("AURA_SUPERVISOR_TEST_OUTPUT").expect("fixture output required");
    let client = SupervisorClient {
        executable: std::path::PathBuf::from(fixture).join("envbox-supervisor.exe"),
        timeout: Duration::from_secs(5),
    };
    let response = client.ensure_started().unwrap();
    assert_eq!(response.status, "Ok");
    std::fs::write(output, serde_json::to_vec(&response).unwrap()).unwrap();
}

#[test]
fn independent_hidden_startup_converges() {
    let _endpoint = PRINCIPAL_ENDPOINT.lock().unwrap();
    use std::os::windows::process::CommandExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::*;
    let fixture = std::env::temp_dir().join(format!("aura-supervisor-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&fixture).unwrap();
    let manager = fixture.join("envbox.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &manager).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_envbox-supervisor"),
        fixture.join("envbox-supervisor.exe"),
    )
    .unwrap();
    let mut clients = Vec::new();
    for index in 0..4 {
        let output = fixture.join(format!("client-{index}.json"));
        let stderr = fixture.join(format!("client-{index}.stderr.log"));
        let stdout = fixture.join(format!("client-{index}.stdout.log"));
        let child = std::process::Command::new(&manager)
            .args([
                "--ignored",
                "--exact",
                "manager_fixture_helper",
                "--nocapture",
            ])
            .env("AURA_SUPERVISOR_TEST_FIXTURE", &fixture)
            .env("AURA_SUPERVISOR_TEST_OUTPUT", &output)
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdout(std::fs::File::create(stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        clients.push((child, output, stderr));
    }
    let end = Instant::now() + Duration::from_secs(10);
    for (child, _, stderr) in &mut clients {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "manager pid={} exit={status} stderr={}\n{}",
                    child.id(),
                    stderr.display(),
                    std::fs::read_to_string(&*stderr).unwrap_or_default()
                );
                break;
            }
            assert!(Instant::now() < end, "manager startup deadline");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let responses: Vec<Response> = clients
        .iter()
        .map(|(_, path, _)| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        .collect();
    assert!(responses.iter().all(|response| response.supervisor_pid
        == responses[0].supervisor_pid
        && response.generation == responses[0].generation));
    // Only this fixture's proven process is terminated; never Stop another owner.
    unsafe {
        let process = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            responses[0].supervisor_pid,
        )
        .unwrap();
        let mut buffer = [0u16; 32768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .unwrap();
        let image = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
        assert_eq!(
            std::fs::canonicalize(image).unwrap(),
            std::fs::canonicalize(fixture.join("envbox-supervisor.exe")).unwrap()
        );
        TerminateProcess(process, 0).unwrap();
        CloseHandle(process).unwrap();
    }
}
