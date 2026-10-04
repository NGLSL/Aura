#![cfg(windows)]

use envbox_launcher::{HostBroker, IpcMessage, SessionTable};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

fn assert_uninjected_fixture_host() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe {
        assert!(GetModuleHandleW(windows::core::w!("envbox-runtime64.dll")).is_err());
        assert!(GetModuleHandleW(windows::core::w!("envbox-runtime32.dll")).is_err());
    }
    println!("fixture_host_pid={} runtime_modules=0", std::process::id());
}

#[test]
fn legacy_ready_cannot_release_entry_over_real_pipe() {
    let table = Arc::new(Mutex::new(SessionTable::new()));
    table.lock().unwrap().bind_pid(std::process::id(), "gate");
    let name = envbox_launcher::session_pipe_name(&uuid::Uuid::new_v4().to_string());
    let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    writeln!(pipe, "RUNTIME_READY pid={}", std::process::id()).unwrap();
    writeln!(
        pipe,
        "STARTUP_GATE_READY pid={} creation_time=0",
        std::process::id()
    )
    .unwrap();
    let mut bytes = [0; 512];
    let n = pipe.read(&mut bytes).unwrap();
    let reply = IpcMessage::decode_line(std::str::from_utf8(&bytes[..n]).unwrap()).unwrap();
    assert!(reply.encode_line().starts_with("ERROR"));
    assert!(!table
        .lock()
        .unwrap()
        .startup_gate_released(std::process::id()));
    // A client cannot submit the receipt ACK before an approved release, nor
    // send a Host-only release message to authorize itself.
    for frame in [
        format!(
            "STARTUP_GATE_RELEASED pid={} creation_time=0",
            std::process::id()
        ),
        format!("STARTUP_RELEASE pid={} creation_time=0", std::process::id()),
        "STARTUP_GATE_READY pid=987654 creation_time=0".into(),
    ] {
        writeln!(pipe, "{frame}").unwrap();
        let n = pipe.read(&mut bytes).unwrap();
        assert!(std::str::from_utf8(&bytes[..n])
            .unwrap()
            .starts_with("ERROR"));
    }
    drop(pipe);
    broker.stop();
}

#[test]
#[ignore = "requires fresh x86/x64 native gate fixture and matching Runtime DLL"]
fn native_entry_marker_requires_approved_runtime() {
    assert_uninjected_fixture_host();
    use envbox_core::{
        DnsMode, DnsProfile, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile,
    };
    use envbox_launcher::{
        build_environment_block, resolve_command, spawn_for_activation, ActivationRequest,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    };
    let dll = std::path::PathBuf::from(std::env::var("AURA_GATE_DLL").expect("fixture DLL"));
    let target = std::env::var("AURA_GATE_TARGET").expect("native no-CRT fixture");
    for scenario in [
        "success",
        "wrong_bundle",
        "gui_wrong_bundle",
        "gui_crt_wrong_bundle",
        "disconnected",
        "idle_host",
        "tls",
    ] {
        let instance = uuid::Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("aura-gate-{instance}"));
        std::fs::create_dir(&root).unwrap();
        let marker = root.join("entry.txt");
        let tls_marker = root.join("tls.txt");
        let profile = EnvironmentProfile {
            id: uuid::Uuid::new_v4(),
            name: "gate-fixture".into(),
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
            environment: Default::default(),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        };
        let table = Arc::new(Mutex::new(SessionTable::new()));
        table.lock().unwrap().set_instance_id(&instance.to_string());
        table
            .lock()
            .unwrap()
            .register_profile_flags(&profile, false, false);
        let name = envbox_launcher::session_pipe_name(&instance.to_string());
        let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
        let mut environment = build_environment_block(
            &std::env::vars().collect(),
            Some(&profile),
            instance,
            profile.id,
            false,
            false,
        );
        environment.insert("ENVBOX_IPC_PIPE".into(), name);
        environment.insert("ENVBOX_STARTUP_GATE".into(), "1".into());
        environment.insert("AURA_GATE_MARKER".into(), marker.to_string_lossy().into());
        environment.insert(
            "AURA_TLS_MARKER".into(),
            tls_marker.to_string_lossy().into(),
        );
        let executable = match scenario {
            "tls" => target.replace("-entry.exe", "-tls.exe"),
            "gui_wrong_bundle" => target.replace("-entry.exe", "-gui.exe"),
            "gui_crt_wrong_bundle" => target.replace("-entry.exe", "-gui-crt.exe"),
            _ => target.clone(),
        };
        let resolved = resolve_command(&executable, None).unwrap();
        let request = ActivationRequest {
            creation_job: None,
            arguments: vec![],
            working_directory: None,
            environment,
            runtime_dll: Some(dll.clone()),
            require_runtime: true,
            webrtc_policy: None,
            browser_locale: None,
            create_new_console: false,
        };
        let child = spawn_for_activation(&resolved, &[], &request).unwrap();
        struct Guard(windows::Win32::Foundation::HANDLE);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    let _ = TerminateProcess(self.0, 1);
                }
            }
        }
        let _guard = Guard(child.process.0);
        table
            .lock()
            .unwrap()
            .bind_pid(child.pid, &profile.id.to_string());
        if !scenario.ends_with("wrong_bundle") {
            table
                .lock()
                .unwrap()
                .expect_runtime(child.pid, &dll)
                .unwrap();
        }
        // caller-requested suspended remains suspended: no loader identity,
        // gate messages, or application marker can arise until ResumeThread.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!marker.exists());
        assert!(table.lock().unwrap().runtime_identity(child.pid).is_none());
        if scenario == "disconnected" {
            broker.stop();
        }
        // Block only this dedicated fixture's broker, never a shared Host.
        // Profile read times out after 3s; the gate's remaining 5s deadline
        // then fails closed. Releasing this lock lets broker shutdown finish.
        let idle = if scenario == "idle_host" {
            let (tx, rx) = std::sync::mpsc::channel();
            let idle_table = table.clone();
            let thread = std::thread::spawn(move || {
                let _locked = idle_table.lock().unwrap();
                tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(9000));
            });
            rx.recv().unwrap();
            Some(thread)
        } else {
            None
        };
        unsafe {
            assert_ne!(ResumeThread(child.thread.as_ref().unwrap().0), u32::MAX);
            assert_eq!(
                WaitForSingleObject(child.process.0, 10000),
                windows::Win32::Foundation::WAIT_OBJECT_0
            );
            if let Some(thread) = idle {
                thread.join().unwrap();
            }
            let mut code = 0;
            GetExitCodeProcess(child.process.0, &mut code).unwrap();
            assert_eq!(code == 0, scenario == "success", "{scenario} code={code}");
            println!(
                "scenario={scenario} code={code} entry={} tls={} released={}",
                marker.exists(),
                tls_marker.exists(),
                table.lock().unwrap().startup_gate_released(child.pid)
            );
        }
        assert_eq!(marker.exists(), scenario == "success", "{scenario}");
        assert!(!tls_marker.exists(), "TLS must not run for rejected image");
        assert_eq!(
            table.lock().unwrap().startup_gate_released(child.pid),
            scenario == "success"
        );
        broker.stop();
        // Delete only the fresh test-owned root, never a supplied application path.
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[ignore = "requires fresh DLL, source runtime staging, and native entry/CRT fixtures"]
fn public_gated_session_publishes_only_approved_entry() {
    assert_uninjected_fixture_host();
    use envbox_core::{
        DnsMode, DnsProfile, EnvironmentProfile, LaunchTarget, LocaleProfile, RegistryProfile,
        SessionState, TimezoneProfile,
    };
    use envbox_launcher::{start_session_gated, SessionStartRequest};
    let dll = std::env::var("AURA_GATE_DLL").expect("fixture DLL");
    let target = std::env::var("AURA_GATE_TARGET").expect("entry fixture");
    // This ignored test runs in a dedicated fresh Host. Avoid parallel tests
    // which mutate the same process-level runtime selection variable.
    struct EnvironmentGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
    let _environment_guard = EnvironmentGuard(
        [
            "ENVBOX_RUNTIME_DLL",
            "ENVBOX_STARTUP_GATE",
            "ENVBOX_IPC_PIPE",
        ]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect(),
    );
    std::env::set_var("ENVBOX_RUNTIME_DLL", dll);
    // Gated startup owns these internal controls. Host pollution must never
    // disable its gate or route this Run to an unrelated bootstrap pipe.
    std::env::set_var("ENVBOX_STARTUP_GATE", "0");
    std::env::set_var("ENVBOX_IPC_PIPE", "aura-nonexistent-polluted-pipe");
    for fixture in ["entry", "return", "tls", "console-crt", "gui", "gui-crt"] {
        let root = std::env::temp_dir().join(format!("aura-public-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let marker = root.join("entry.txt");
        let tls_marker = root.join("tls.txt");
        let profile = EnvironmentProfile {
            id: uuid::Uuid::new_v4(),
            name: "public-gate-fixture".into(),
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
            environment: [
                ("AURA_GATE_MARKER".into(), marker.to_string_lossy().into()),
                (
                    "AURA_TLS_MARKER".into(),
                    tls_marker.to_string_lossy().into(),
                ),
            ]
            .into(),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        };
        let request = SessionStartRequest {
            application_id: uuid::Uuid::new_v4(),
            launch: LaunchTarget::Executable {
                path: target
                    .replace("-entry.exe", &format!("-{fixture}.exe"))
                    .into(),
            },
            arguments: vec![],
            working_directory: None,
            profile: Some(profile),
            inherit_children: false,
            audit: false,
        };
        let outcome = start_session_gated(request);
        match outcome {
            Ok(mut handle) => {
                assert_eq!(handle.session.state, SessionState::Running);
                assert!(
                    handle
                        .attached
                        .as_ref()
                        .expect("Runtime attached")
                        .handshake_ok
                );
                let observed = handle.runtime_identity().expect("actual Runtime identity");
                assert!(observed.identity.config_complete);
                assert_eq!(handle.wait_root().unwrap(), 0);
                assert!(marker.exists());
                assert!(
                    matches!(
                        fixture,
                        "entry" | "return" | "console-crt" | "gui" | "gui-crt"
                    ),
                    "unsupported target was unexpectedly accepted: {fixture}"
                );
                println!("public_fixture={fixture} accepted=true marker=true handshake=true generation={} module={}", observed.identity.creation_time, observed.identity.module_path);
            }
            Err(error) => {
                assert!(
                    !marker.exists(),
                    "failed startup already executed {fixture}"
                );
                assert!(
                    !tls_marker.exists(),
                    "rejected TLS target executed callback"
                );
                assert!(
                    !matches!(
                        fixture,
                        "entry" | "return" | "console-crt" | "gui" | "gui-crt"
                    ),
                    "verified supported entry refused: {fixture}: {error}"
                );
                println!("public_fixture={fixture} accepted=false marker=false error={error}");
            }
        }
        // Each root belongs solely to this dedicated fixture run.
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[ignore = "requires stable current typed Runtime pair and x64/x86 native fixtures on an uninjected Host"]
fn cmd_intermediary_preserves_child_identity_gate_and_job() {
    assert_uninjected_fixture_host();
    use envbox_core::{
        DnsMode, DnsProfile, EnvironmentProfile, LaunchTarget, LocaleProfile, RegistryProfile,
        TimezoneProfile,
    };
    use envbox_launcher::{start_session_gated, SessionStartRequest};
    let bundle = std::path::PathBuf::from(
        std::env::var("AURA_MATRIX_BUNDLE").expect("stable pair directory"),
    );
    let fixtures = std::path::PathBuf::from(
        std::env::var("AURA_MATRIX_FIXTURES").expect("target containing gate-fixture32/64"),
    );
    let windows_dir = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap());
    struct RuntimeGuard(Option<std::ffi::OsString>);
    impl Drop for RuntimeGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("ENVBOX_RUNTIME_DLL", v),
                None => std::env::remove_var("ENVBOX_RUNTIME_DLL"),
            }
        }
    }
    let _runtime_guard = RuntimeGuard(std::env::var_os("ENVBOX_RUNTIME_DLL"));
    for (parent_arch, child_arch) in [(64, 64), (64, 32), (32, 32), (32, 64)] {
        let root = std::env::temp_dir().join(format!("aura-cmd-matrix-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let marker = root.join("entry.txt");
        let facts = root.join("process.txt");
        let profile = EnvironmentProfile {
            id: uuid::Uuid::new_v4(),
            name: "cmd-child-fixture".into(),
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
            environment: [
                ("AURA_GATE_MARKER".into(), marker.to_string_lossy().into()),
                (
                    "AURA_GATE_PID_MARKER".into(),
                    facts.to_string_lossy().into(),
                ),
                ("AURA_GATE_WAIT_MS".into(), "15000".into()),
            ]
            .into(),
            registry: RegistryProfile::default(),
            browser: Default::default(),
        };
        let profile_id = profile.id;
        let cmd = windows_dir
            .join(if parent_arch == 64 {
                "System32"
            } else {
                "SysWOW64"
            })
            .join("cmd.exe");
        let fixture = fixtures
            .join(format!("gate-fixture{child_arch}"))
            .join("Release/envbox-startup-gate-entry.exe");
        assert!(
            !fixture.to_string_lossy().contains(' '),
            "this bounded CMD fixture uses an unambiguous space-free path"
        );
        std::env::set_var(
            "ENVBOX_RUNTIME_DLL",
            bundle.join(format!("envbox-runtime{parent_arch}.dll")),
        );
        let request = SessionStartRequest {
            application_id: uuid::Uuid::new_v4(),
            launch: LaunchTarget::Executable { path: cmd },
            arguments: vec!["/d".into(), "/c".into(), fixture.to_string_lossy().into()],
            working_directory: None,
            profile: Some(profile),
            inherit_children: true,
            audit: false,
        };
        let outcome = start_session_gated(request);
        let mut handle = match outcome {
            Ok(handle) => handle,
            Err(error) => {
                assert!(!marker.exists(), "failed root already entered child");
                panic!("matrix {parent_arch}->{child_arch} root rejected: {error}");
            }
        };
        struct StopGuard<'a>(&'a envbox_launcher::InstanceJob);
        impl Drop for StopGuard<'_> {
            fn drop(&mut self) {
                let _ = self.0.terminate();
            }
        }
        let job = handle.job.as_ref().expect("real Job");
        let _stop = StopGuard(job);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let data = loop {
            if let Ok(data) = std::fs::read_to_string(&facts) {
                if data.split_whitespace().count() == 2 {
                    break data;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "matrix {parent_arch}->{child_arch} child never entered or completed its facts"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let fields: Vec<_> = data.split_whitespace().collect();
        let child_pid = u32::from_str_radix(fields[0], 16).unwrap();
        let generation = u64::from_str_radix(fields[1], 16).unwrap();
        let members = job.stats().unwrap().process_ids;
        assert!(members.contains(&handle.instance.root_pid));
        assert!(members.contains(&child_pid));
        let table = handle.broker.as_ref().unwrap().table();
        let locked = table.lock().unwrap();
        let root_identity = locked.validate_runtime(handle.instance.root_pid).unwrap();
        let child_identity = locked.validate_runtime(child_pid).unwrap();
        assert_eq!(child_identity.identity.creation_time, generation);
        assert_eq!(
            child_identity.identity.actual_profile,
            root_identity.identity.actual_profile
        );
        assert_eq!(
            locked.profile_of(child_pid),
            Some(profile_id.to_string().as_str())
        );
        assert!(locked.startup_gate_released(child_pid));
        assert!(child_identity
            .identity
            .module_path
            .ends_with(&format!("envbox-runtime{child_arch}.dll")));
        println!("cmd_matrix={parent_arch}->{child_arch} root_pid={} child_pid={child_pid} generation={generation} child_gate=true profile_matches=true job_members={members:?}", handle.instance.root_pid);
        drop(locked);
        job.terminate().unwrap();
        drop(_stop);
        assert_eq!(handle.wait_root().unwrap(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
#[ignore = "requires frozen DLL pair and native fixture on an uninjected Windows Host"]
fn creation_job_membership_precedes_resume_and_injection_failure_is_clean() {
    assert_uninjected_fixture_host();
    use envbox_core::{
        DnsMode, DnsProfile, EnvironmentProfile, LocaleProfile, RegistryProfile, TimezoneProfile,
    };
    use envbox_launcher::{
        build_environment_block, resolve_command, spawn_for_activation, ActivationRequest,
        InstanceJob,
    };
    use windows::Win32::System::Threading::{ResumeThread, WaitForSingleObject};
    let bundle = std::path::PathBuf::from(std::env::var("AURA_MATRIX_BUNDLE").unwrap());
    let fixtures = std::path::PathBuf::from(std::env::var("AURA_MATRIX_FIXTURES").unwrap());
    for arch in [64, 32] {
        for mode in ["host", "profile", "invalid_dll", "tls"] {
            let instance = uuid::Uuid::new_v4();
            let root = std::env::temp_dir().join(format!("aura-atomic-job-{instance}"));
            std::fs::create_dir(&root).unwrap();
            let marker = root.join("entry.txt");
            let profile = EnvironmentProfile {
                id: uuid::Uuid::new_v4(),
                name: "atomic-job-fixture".into(),
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
                environment: [("AURA_GATE_MARKER".into(), marker.to_string_lossy().into())].into(),
                registry: RegistryProfile::default(),
                browser: Default::default(),
            };
            let table = Arc::new(Mutex::new(SessionTable::new()));
            table.lock().unwrap().set_instance_id(&instance.to_string());
            table
                .lock()
                .unwrap()
                .register_profile_flags(&profile, false, false);
            let name = envbox_launcher::session_pipe_name(&instance.to_string());
            let mut broker = HostBroker::start_on(table.clone(), name.clone()).unwrap();
            let mut environment = build_environment_block(
                &std::env::vars().collect(),
                Some(&profile),
                instance,
                profile.id,
                false,
                false,
            );
            environment.insert("AURA_GATE_MARKER".into(), marker.to_string_lossy().into());
            environment.insert("ENVBOX_IPC_PIPE".into(), name);
            if matches!(mode, "profile" | "tls") {
                environment.insert("ENVBOX_STARTUP_GATE".into(), "1".into());
            }
            let dll = bundle.join(format!("envbox-runtime{arch}.dll"));
            let invalid = root.join("broken.dll");
            let job = InstanceJob::create().unwrap();
            struct Stop<'a>(&'a InstanceJob);
            impl Drop for Stop<'_> {
                fn drop(&mut self) {
                    let _ = self.0.terminate();
                }
            }
            let _stop = Stop(&job);
            let name = if mode == "tls" { "tls" } else { "entry" };
            let executable = fixtures.join(format!(
                "gate-fixture{arch}/Release/envbox-startup-gate-{name}.exe"
            ));
            let resolved = resolve_command(executable.to_str().unwrap(), None).unwrap();
            let request = ActivationRequest {
                creation_job: Some(job.creation_assignment()),
                arguments: vec![],
                working_directory: None,
                environment,
                runtime_dll: match mode {
                    "host" => None,
                    "invalid_dll" => Some(invalid),
                    _ => Some(dll.clone()),
                },
                require_runtime: mode != "host",
                webrtc_policy: None,
                browser_locale: None,
                create_new_console: false,
            };
            let outcome = spawn_for_activation(&resolved, &[], &request);
            if mode == "invalid_dll" {
                assert!(outcome.is_err(), "invalid DLL unexpectedly injected");
                assert!(
                    job.stats().unwrap().process_ids.is_empty(),
                    "failed injection left a Job process"
                );
                assert!(!marker.exists());
                println!("atomic_job={arch}/{mode} refused=true members=[] entry=false");
            } else {
                let child = outcome.unwrap();
                assert!(child.suspended);
                assert_eq!(job.stats().unwrap().process_ids, vec![child.pid]);
                assert!(!marker.exists());
                if matches!(mode, "profile" | "tls") {
                    table
                        .lock()
                        .unwrap()
                        .bind_pid(child.pid, &profile.id.to_string());
                    table
                        .lock()
                        .unwrap()
                        .expect_runtime(child.pid, &dll)
                        .unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                assert!(!marker.exists(), "suspended contract violated");
                println!(
                    "atomic_job={arch}/{mode} pid={} before_resume_members={:?} entry=false",
                    child.pid,
                    job.stats().unwrap().process_ids
                );
                unsafe {
                    assert_ne!(ResumeThread(child.thread.as_ref().unwrap().0), u32::MAX);
                    assert_eq!(
                        WaitForSingleObject(child.process.0, 10000),
                        windows::Win32::Foundation::WAIT_OBJECT_0
                    );
                }
                assert_eq!(marker.exists(), mode != "tls");
                if mode == "tls" {
                    assert!(
                        job.stats().unwrap().process_ids.is_empty(),
                        "DLL initialization failure retained an active Job process"
                    );
                    assert!(!table.lock().unwrap().startup_gate_released(child.pid));
                    println!(
                        "atomic_job={arch}/tls initialization_refused=true members=[] entry=false"
                    );
                }
                if mode == "profile" {
                    assert!(table.lock().unwrap().startup_gate_released(child.pid));
                }
            }
            broker.stop();
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
