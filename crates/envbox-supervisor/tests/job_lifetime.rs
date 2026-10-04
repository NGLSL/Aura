#![cfg(windows)]

#[test]
#[ignore = "requires native wait fixture and an uninjected host; records kernel Job last-handle behavior"]
fn last_owner_close_preserves_target_but_removes_named_control() {
    use envbox_launcher::InstanceJob;
    use std::os::windows::process::CommandExt;
    let target = std::env::var("AURA_SUPERVISOR_RUN_TARGET").expect("native wait fixture required");
    let root = std::env::temp_dir().join(format!("aura-job-lifetime-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let mut child = std::process::Command::new(target)
        .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
        .env("AURA_GATE_MARKER", root.join("entry.txt"))
        .env("AURA_GATE_WAIT_MS", "30000")
        .spawn()
        .unwrap();
    let name = format!("Local\\AuraJobLifetime-{}", uuid::Uuid::new_v4());
    let mut job = InstanceJob::create_named_exclusive(&name).unwrap();
    job.assign_pid(child.id()).unwrap();
    assert!(job.stats().unwrap().process_ids.contains(&child.id()));
    drop(job);
    assert!(
        child.try_wait().unwrap().is_none(),
        "last-owner close must not kill target"
    );
    let error = InstanceJob::open_named(&name).err();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        matches!(error, Some(envbox_launcher::job::JobError::Open(2))),
        "without a query-handle escrow the native name cannot be reopened: {error:?}"
    );
    println!("last_external_job_handle_closed target_alive=true reopen_named=false win32_error=2; explicit query-handle escrow is required");
}
