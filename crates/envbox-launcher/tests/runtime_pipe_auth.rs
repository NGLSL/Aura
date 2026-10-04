#![cfg(windows)]

use envbox_launcher::{HostBroker, IpcMessage, SessionTable};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

// Public transport seam: this is an actual local Windows pipe client, not
// SessionTable::handle (which is intentionally a trusted host-only interface).
#[test]
fn runtime_pipe_cannot_register_or_bind_other_processes() {
    let table = Arc::new(Mutex::new(SessionTable::new()));
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    writeln!(client, "BIND_PID pid=987654 profile_id=stolen parent_pid=0").unwrap();
    // A subsequent request flushes earlier messages through the same pipe.
    writeln!(
        client,
        "GET_PROFILE pid={} profile_id=stolen",
        std::process::id()
    )
    .unwrap();
    let mut response = [0; 1024];
    assert!(client.read(&mut response).unwrap() > 0);
    assert_eq!(table.lock().unwrap().profile_of(987654), None);
    drop(client);
    broker.stop();
}

#[test]
fn profile_hint_cannot_rebind_a_real_pipe_client() {
    let table = Arc::new(Mutex::new(SessionTable::new()));
    table.lock().unwrap().bind_pid(std::process::id(), "owned");
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    writeln!(
        client,
        "GET_PROFILE pid={} profile_id=other",
        std::process::id()
    )
    .unwrap();
    let mut response = [0; 1024];
    let bytes = client.read(&mut response).unwrap();
    assert!(bytes > 0);
    let reply = IpcMessage::decode_line(std::str::from_utf8(&response[..bytes]).unwrap()).unwrap();
    assert_ne!(reply.name(), "PROFILE");
    assert_eq!(
        table.lock().unwrap().profile_of(std::process::id()),
        Some("owned")
    );
    drop(client);
    broker.stop();
}

#[test]
fn idle_pipe_client_cannot_block_broker_stop() {
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let table = Arc::new(Mutex::new(SessionTable::new()));
    let mut broker = HostBroker::start_on(table, name.clone()).unwrap();
    let client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    let started = std::time::Instant::now();
    broker.stop();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    drop(client);
}

#[test]
fn pipe_rejects_spoofed_pid_and_malformed_frames() {
    let table = Arc::new(Mutex::new(SessionTable::new()));
    table.lock().unwrap().bind_pid(std::process::id(), "owned");
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    for frame in ["RUNTIME_READY pid=987654", "GET_PROFILE pid=not-a-number"] {
        writeln!(client, "{frame}").unwrap();
        let mut response = [0; 1024];
        let n = client.read(&mut response).unwrap();
        let reply = IpcMessage::decode_line(std::str::from_utf8(&response[..n]).unwrap()).unwrap();
        assert_eq!(reply.encode_line().split_whitespace().next(), Some("ERROR"));
    }
    assert!(table.lock().unwrap().events.is_empty());
    writeln!(
        client,
        "GET_PROFILE pid={} profile_id=\"owned",
        std::process::id()
    )
    .unwrap();
    let mut response = [0; 1024];
    let n = client.read(&mut response).unwrap();
    assert!(std::str::from_utf8(&response[..n])
        .unwrap()
        .contains("malformed_message"));
    assert_eq!(
        table.lock().unwrap().profile_of(std::process::id()),
        Some("owned")
    );
    drop(client);
    broker.stop();
}

#[test]
fn oversized_unterminated_frame_closes_connection() {
    let table = Arc::new(Mutex::new(SessionTable::new()));
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    let _ = client.write_all(&vec![b'x'; 32769]);
    let mut response = [0; 16];
    assert!(matches!(client.read(&mut response), Ok(0) | Err(_)));
    assert!(table.lock().unwrap().events.is_empty());
    drop(client);
    broker.stop();
}

#[test]
fn legacy_ready_and_forged_generation_cannot_confirm_runtime() {
    let table = Arc::new(Mutex::new(SessionTable::new()));
    table.lock().unwrap().bind_pid(std::process::id(), "owned");
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    writeln!(client, "RUNTIME_READY pid={}", std::process::id()).unwrap();
    writeln!(client, "RUNTIME_IDENTITY pid={} creation_time=0 protocol=1 version={} module_path=none actual_profile=PROFILE config_complete=1", std::process::id(), env!("CARGO_PKG_VERSION")).unwrap();
    let mut response = [0; 1024];
    let n = client.read(&mut response).unwrap();
    let line = std::str::from_utf8(&response[..n]).unwrap();
    assert!(line.contains("runtime_protocol_or_generation_mismatch"));
    assert!(table
        .lock()
        .unwrap()
        .runtime_identity(std::process::id())
        .is_none());
    assert!(table
        .lock()
        .unwrap()
        .validate_runtime(std::process::id())
        .is_err());
    assert_eq!(
        table.lock().unwrap().profile_of(std::process::id()),
        Some("owned")
    );
    drop(client);
    broker.stop();
}

#[test]
fn versioned_dns_wire_preserves_order_and_rejects_partial_snapshots() {
    let wire = "PROFILE profile_id=p instance_id=i locale_name=en-US ui_language=en-US region=US tz_windows=UTC tz_iana=Etc/UTC inherit_children=1 audit=0 webrtc=host dns_mode=1 dns_config_version=1 dns_strict=1 dns_upstream_count=4 dns_upstream_0_type=doh dns_upstream_0_url=https://resolver.example.test/dns-query dns_upstream_0_bootstrap_count=1 dns_upstream_0_bootstrap_0=1.1.1.1 dns_upstream_1_type=dot dns_upstream_1_address=1.1.1.1 dns_upstream_1_port=853 dns_upstream_1_server_name=resolver.example.test dns_upstream_2_type=tcp dns_upstream_2_address=1.1.1.1 dns_upstream_2_port=5353 dns_upstream_3_type=udp dns_upstream_3_address=1.1.1.1 dns_upstream_3_port=53";
    let profile =
        envbox_launcher::message_to_profile(&IpcMessage::decode_line(wire).unwrap()).unwrap();
    let upstreams = profile.dns.effective_upstreams();
    assert!(matches!(upstreams[0], envbox_core::DnsUpstream::Doh { .. }));
    assert!(matches!(
        upstreams[1],
        envbox_core::DnsUpstream::Dot { port: 853, .. }
    ));
    assert!(matches!(
        upstreams[2],
        envbox_core::DnsUpstream::Tcp { port: 5353, .. }
    ));
    assert!(matches!(
        upstreams[3],
        envbox_core::DnsUpstream::Udp { port: 53, .. }
    ));
    assert!(profile.dns.strict);
    for broken in [
        wire.replace(" dns_config_version=1", ""),
        wire.replace(" dns_strict=1", ""),
        wire.replace("dns_mode=1", "dns_mode=unknown"),
        wire.replace("dns_upstream_count=4", "dns_upstream_count=3"),
        format!("{wire} dns_server=8.8.8.8"),
        format!("{wire} dns_upstream_4_type=udp"),
    ] {
        assert!(IpcMessage::decode_line(&broken).is_err(), "{broken}");
    }
}

#[test]
#[ignore = "child fixture, invoked only by injected_runtime_reports_actual_identity"]
fn identity_fixture() {
    if std::env::var("AURA_IDENTITY_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    assert_eq!(
        std::env::var("AURA_IDENTITY_VALUE").unwrap(),
        "profile-value"
    );
    std::thread::sleep(std::time::Duration::from_secs(3));
}

#[test]
#[ignore = "requires explicit freshly built DLL and an uninjected Windows runner"]
fn injected_runtime_reports_actual_identity() {
    use envbox_core::{
        DnsMode, DnsProfile, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile,
    };
    use envbox_launcher::{
        build_environment_block, resolve_command, spawn_for_activation, ActivationRequest,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    };
    let dll =
        std::path::PathBuf::from(std::env::var("AURA_IDENTITY_DLL").expect("explicit fixture DLL"));
    let instance = uuid::Uuid::new_v4();
    let child_target = std::env::var("AURA_CHILD_TARGET").ok();
    let inherit = child_target.is_some();
    let mut profile = EnvironmentProfile {
        id: uuid::Uuid::new_v4(),
        name: "identity-fixture".into(),
        locale: LocaleProfile {
            locale_name: "en-US".into(),
            ui_language: "en-US".into(),
            region: "US".into(),
        },
        timezone: TimezoneProfile {
            windows_id: "Pacific Standard Time".into(),
            iana_id: "America/Los_Angeles".into(),
        },
        dns: DnsProfile {
            mode: DnsMode::Host,
            servers: vec![],
            ..Default::default()
        },
        environment: [("AURA_IDENTITY_VALUE".into(), "profile-value".into())].into(),
        registry: RegistryProfile::default(),
        browser: Default::default(),
    };
    if std::env::var_os("AURA_DNS_WIRE_MATRIX").is_some() {
        use envbox_core::DnsUpstream;
        profile.dns = DnsProfile::typed(
            DnsMode::Host,
            false,
            vec![
                DnsUpstream::Doh {
                    url: format!(
                        "https://resolver.example.test/dns-query/{}",
                        "a".repeat(600)
                    ),
                    bootstrap_ips: vec!["1.1.1.1".parse().unwrap()],
                },
                DnsUpstream::Dot {
                    address: "1.1.1.1".parse().unwrap(),
                    port: 853,
                    server_name: "resolver.example.test".into(),
                },
                DnsUpstream::Tcp {
                    address: "1.1.1.1".parse().unwrap(),
                    port: 5353,
                },
                DnsUpstream::Udp {
                    address: "1.1.1.1".parse().unwrap(),
                    port: 53,
                },
            ],
        );
    }
    let table = Arc::new(Mutex::new(SessionTable::new()));
    table.lock().unwrap().set_instance_id(&instance.to_string());
    table
        .lock()
        .unwrap()
        .register_profile_flags(&profile, inherit, false);
    let name = envbox_launcher::session_pipe_name(&instance.to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let host = std::env::vars().collect();
    let mut environment =
        build_environment_block(&host, Some(&profile), instance, profile.id, inherit, false);
    environment.insert("ENVBOX_IPC_PIPE".into(), name);
    environment.insert("AURA_IDENTITY_FIXTURE".into(), "1".into());
    let escrow_job = std::env::var_os("AURA_RECOVERY_ESCROW")
        .map(|_| format!("Local\\AuraRecoveryFixture-{instance}"));
    if let Some(name) = &escrow_job {
        environment.insert("ENVBOX_RECOVERY_JOB_NAME".into(), name.clone());
    }
    let marker = std::env::temp_dir().join(format!("aura-child-identity-{instance}.txt"));
    let legacy_child = std::env::var_os("AURA_CHILD_LEGACY").is_some();
    if let Some(target) = &child_target {
        environment.insert("AURA_CHILD_TARGET".into(), target.clone());
        environment.insert("AURA_CHILD_MARKER".into(), marker.to_string_lossy().into());
        if !legacy_child {
            environment.insert("ENVBOX_STARTUP_GATE".into(), "1".into());
        }
        for key in ["AURA_CHILD_API", "AURA_CHILD_SUSPENDED"] {
            if let Ok(value) = std::env::var(key) {
                environment.insert(key.into(), value);
            }
        }
    }
    let executable = std::env::var("AURA_IDENTITY_TARGET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::current_exe().unwrap());
    let resolved = resolve_command(executable.to_str().unwrap(), None).unwrap();
    let arguments = if std::env::var_os("AURA_IDENTITY_TARGET").is_some() {
        vec![]
    } else {
        vec![
            "--ignored".into(),
            "--exact".into(),
            "identity_fixture".into(),
            "--nocapture".into(),
        ]
    };
    let request = ActivationRequest {
        creation_job: None,
        arguments: arguments.clone(),
        working_directory: None,
        environment,
        runtime_dll: Some(dll.clone()),
        require_runtime: true,
        webrtc_policy: None,
        browser_locale: None,
        create_new_console: false,
    };
    let child = spawn_for_activation(&resolved, &arguments, &request).unwrap();
    let mut job = if let Some(name) = &escrow_job {
        envbox_launcher::InstanceJob::create_named_exclusive(name).unwrap()
    } else {
        envbox_launcher::InstanceJob::create().unwrap()
    };
    job.assign_pid(child.pid).unwrap();
    struct ChildGuard(windows::Win32::Foundation::HANDLE);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = TerminateProcess(self.0, 1);
            }
        }
    }
    let guard = ChildGuard(child.process.0);
    table
        .lock()
        .unwrap()
        .bind_pid(child.pid, &profile.id.to_string());
    table
        .lock()
        .unwrap()
        .expect_runtime(child.pid, &dll)
        .unwrap();
    let original_generation = unsafe {
        use windows::Win32::Foundation::FILETIME;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        windows::Win32::System::Threading::GetProcessTimes(
            child.process.0,
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .unwrap();
        assert_ne!(ResumeThread(child.thread.as_ref().unwrap().0), u32::MAX);
        ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while table
        .lock()
        .unwrap()
        .validate_runtime_generation(child.pid, original_generation)
        .is_err()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let locked = table.lock().unwrap();
    if locked
        .validate_runtime_generation(child.pid, original_generation)
        .is_err()
    {
        let mut code = 0;
        unsafe {
            GetExitCodeProcess(child.process.0, &mut code).unwrap();
        }
        println!("identity_missing_exit={code} events={:?}", locked.events);
    }
    let observed = locked
        .validate_runtime_generation(child.pid, original_generation)
        .expect("actual module/config/hook identity accepted");
    println!(
        "pid={} generation={} module={} module_sha256={} config_sha256={} hooks={:?}",
        child.pid,
        observed.identity.creation_time,
        observed.identity.module_path,
        observed.module_sha256,
        observed.config_sha256,
        observed.identity.hooks
    );
    assert!(observed.identity.config_complete);
    assert!(observed
        .identity
        .actual_profile
        .contains(&instance.to_string()));
    let sealed_identity = observed.clone();
    drop(locked);
    if std::env::var_os("AURA_RECOVERY_FIXTURE").is_some() {
        use envbox_launcher::request_runtime_reconnect;
        use std::{path::Path, time::Duration};
        let generation = sealed_identity.identity.creation_time;
        let path = Path::new(&sealed_identity.identity.module_path);
        let hash = &sealed_identity.module_sha256;
        if let Some(name) = &escrow_job {
            drop(job);
            job = envbox_launcher::InstanceJob::open_named(name)
                .expect("Runtime query handle preserves Job after last Host owner drops");
            job.verify_tracking_limits().unwrap();
            assert!(job.stats().unwrap().process_ids.contains(&child.pid));
            println!("runtime_job_escrow_preserved_pid={} job={name}", child.pid);
        }
        let caps = envbox_launcher::read_runtime_capabilities(path)
            .expect("new Runtime advertises offline capabilities");
        assert!(caps.entry_gate && caps.reconnect && caps.dns_udp && caps.dns_tcp);
        assert_eq!(caps.profile_dns_schema, 1);
        let strict_udp =
            DnsProfile::from_servers(DnsMode::VirtualView, vec!["1.1.1.1".parse().unwrap()]);
        envbox_launcher::validate_runtime_for_profile(path, &strict_udp, true).unwrap();
        if let Ok(old_path) = std::env::var("AURA_RECOVERY_OLD_DLL") {
            use sha2::{Digest, Sha256};
            let old = Path::new(&old_path);
            let old_hash = format!("{:x}", Sha256::digest(std::fs::read(old).unwrap()));
            assert!(envbox_launcher::read_runtime_capabilities(old).is_err());
            assert!(envbox_launcher::validate_runtime_for_profile(old, &strict_udp, true).is_err());
            let missing = request_runtime_reconnect(
                child.pid,
                generation,
                old,
                &old_hash,
                Duration::from_secs(4),
            )
            .unwrap_err()
            .to_string();
            assert!(missing.contains("no reconnect export"), "{missing}");
        }
        broker.stop();
        drop(broker); // Release the stopped Host's pipe-name ownership lease.
        assert!(request_runtime_reconnect(
            child.pid,
            generation + 1,
            path,
            hash,
            Duration::from_secs(4)
        )
        .is_err());
        assert!(request_runtime_reconnect(
            child.pid,
            generation,
            path,
            "wrong",
            Duration::from_secs(4)
        )
        .is_err());
        assert!(request_runtime_reconnect(
            child.pid,
            generation,
            path,
            hash,
            Duration::from_millis(20)
        )
        .is_err());
        let mut alive = 0;
        unsafe {
            GetExitCodeProcess(child.process.0, &mut alive).unwrap();
        }
        assert_eq!(alive, 259, "timeout leaves original process alive");
        std::thread::sleep(Duration::from_millis(3100));
        let build_registry = |changed: bool| {
            let mut snapshot = profile.clone();
            if changed {
                snapshot
                    .environment
                    .insert("AURA_IDENTITY_VALUE".into(), "changed".into());
            }
            let mut registry = SessionTable::new();
            registry.set_instance_id(&instance.to_string());
            registry.register_profile_flags(&snapshot, inherit, false);
            registry.bind_pid(child.pid, &snapshot.id.to_string());
            registry.expect_runtime(child.pid, path).unwrap();
            Arc::new(Mutex::new(registry))
        };
        let changed = build_registry(true);
        let mut changed_broker = HostBroker::start_on(
            changed.clone(),
            envbox_launcher::session_pipe_name(&instance.to_string()),
        )
        .unwrap();
        assert!(request_runtime_reconnect(
            child.pid,
            generation,
            path,
            hash,
            Duration::from_secs(4)
        )
        .is_err());
        assert!(!changed.lock().unwrap().runtime_reconfirmed(child.pid));
        changed_broker.stop();
        drop(changed_broker);
        let recovered = build_registry(false);
        broker = HostBroker::start_on(
            recovered.clone(),
            envbox_launcher::session_pipe_name(&instance.to_string()),
        )
        .unwrap();
        assert!(!recovered.lock().unwrap().runtime_reconfirmed(child.pid));
        request_runtime_reconnect(child.pid, generation, path, hash, Duration::from_secs(4))
            .expect("dedicated export performs fresh authenticated reconfirmation");
        assert!(recovered.lock().unwrap().runtime_reconfirmed(child.pid));
        assert_eq!(
            recovered
                .lock()
                .unwrap()
                .validate_runtime(child.pid)
                .unwrap()
                .config_sha256,
            sealed_identity.config_sha256
        );
        println!(
            "recovery_confirmed_pid={} generation={generation} snapshot_sha256={}",
            child.pid, sealed_identity.config_sha256
        );
    }
    if child_target.is_some() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        let mut verified = false;
        while std::time::Instant::now() < deadline {
            let locked = table.lock().unwrap();
            let child_pid = locked.events.iter().find_map(|message| match message {
                IpcMessage::RegisterChild { child_pid, .. } => Some(*child_pid),
                _ => None,
            });
            if let Some(pid) = child_pid {
                if (legacy_child || locked.startup_gate_released(pid))
                    && locked.validate_runtime(pid).is_ok()
                {
                    if legacy_child {
                        assert!(
                            !locked.startup_gate_released(pid),
                            "legacy binding does not claim entry gate approval"
                        );
                    }
                    let id = locked.runtime_identity(pid).unwrap();
                    println!(
                        "verified_child_pid={pid} module={} config_sha256={}",
                        id.identity.module_path, id.config_sha256
                    );
                    assert_eq!(
                        id.config_sha256,
                        locked.runtime_identity(child.pid).unwrap().config_sha256
                    );
                    assert!(
                        job.stats().unwrap().process_ids.contains(&pid),
                        "OS Job membership independently includes the child"
                    );
                    assert_eq!(locked.events.iter().filter(|event| matches!(event, IpcMessage::RegisterChild { child_pid, .. } if *child_pid == pid)).count(), 1, "repeated registration is idempotent");
                    verified = true;
                    break;
                }
            }
            drop(locked);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !verified {
            let mut code = 0;
            unsafe {
                GetExitCodeProcess(child.process.0, &mut code).unwrap();
            }
            println!(
                "child_unverified_parent_exit={code} events={:?}",
                table.lock().unwrap().events
            );
        }
        assert!(
            verified,
            "child requires binding ACK and actual identity before entry"
        );
    }
    unsafe {
        assert_eq!(
            WaitForSingleObject(
                child.process.0,
                if std::env::var_os("AURA_RECOVERY_FIXTURE").is_some() {
                    15000
                } else {
                    5000
                }
            ),
            windows::Win32::Foundation::WAIT_OBJECT_0
        );
        let mut code = 1;
        GetExitCodeProcess(child.process.0, &mut code).unwrap();
        assert_eq!(
            code, 0,
            "fixture independently observed the expected Profile"
        );
    }
    drop(guard);
    if std::env::var_os("AURA_SHORT_IDENTITY_FIXTURE").is_some() {
        let mut registry = table.lock().unwrap();
        registry
            .validate_runtime_generation(child.pid, original_generation)
            .expect("original owned generation keeps its authenticated observation after exit");
        assert!(registry
            .validate_runtime_generation(child.pid, original_generation + 1)
            .is_err());
        registry.handle(&IpcMessage::ProcessExited {
            pid: child.pid,
            exit_code: 0,
        });
        assert!(registry.runtime_identity(child.pid).is_none());
        registry
            .validate_runtime_generation(child.pid, original_generation)
            .expect("exit notification preserves only the original root observation");
    }
    broker.stop();
    if child_target.is_some() {
        assert!(marker.exists(), "controlled child entry ran");
        std::fs::remove_file(marker).unwrap();
    }
}
