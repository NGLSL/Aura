//! Environment Session Broker (V0.3 ticket 43).
//!
//! Long-lived Host process that owns the Session Registry (PID → Profile)
//! and serves Runtime IPC Bootstrap on a Named Pipe. Protocol is the same
//! line contract as `envbox-launcher::ipc` (HELLO / GET_PROFILE / PROFILE /
//! REGISTER_PROFILE / BIND_PID / PROCESS_* / RUNTIME_READY / HOOK_ERROR).

use envbox_launcher::ipc::{IpcMessage, SessionTable};
use envbox_launcher::ipc_server::{HostBroker, SharedTable};
use std::sync::{Arc, Mutex};

pub use envbox_launcher::ipc as protocol;
pub use envbox_launcher::ipc_server as server;

/// Session Registry handle shared with the pipe server.
pub type Registry = SharedTable;

/// Create an empty Session Registry.
pub fn new_registry() -> Registry {
    Arc::new(Mutex::new(SessionTable::new()))
}

/// Bind the registry into a Named Pipe server.
pub fn serve(registry: Registry, pipe_name: String) -> std::io::Result<HostBroker> {
    HostBroker::start_on(registry, pipe_name)
}

/// Host-side helpers used by CLI/GUI to populate the registry over the pipe
/// or in-process (same message shapes).
pub fn register_profile_line(registry: &Registry, line: &str) -> Result<(), String> {
    let msg = IpcMessage::decode_line(line).map_err(|e| e.to_string())?;
    registry.lock().unwrap().handle(&msg);
    Ok(())
}

pub fn bind_pid(registry: &Registry, pid: u32, profile_id: &str, parent_pid: u32) {
    let msg = IpcMessage::BindPid {
        pid,
        profile_id: profile_id.to_string(),
        parent_pid,
    };
    registry.lock().unwrap().handle(&msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_serves_registered_profile_by_pid() {
        let reg = new_registry();
        let line = "REGISTER_PROFILE profile_id=p1 instance_id=i locale_name=en-US ui_language=en-US region=US tz_windows=PST tz_iana=UTC inherit_children=1 audit=0 dns_mode=0";
        register_profile_line(&reg, line).unwrap();
        bind_pid(&reg, 42, "p1", 0);
        let reply = {
            let mut t = reg.lock().unwrap();
            t.handle(&IpcMessage::GetProfile {
                pid: 42,
                profile_id: String::new(),
            })
        };
        assert!(reply.is_some());
        let msg = reply.unwrap();
        assert_eq!(msg.name(), "PROFILE");
    }

    #[test]
    fn child_inherits_parent_binding() {
        let reg = new_registry();
        register_profile_line(
            &reg,
            "REGISTER_PROFILE profile_id=p1 instance_id=i locale_name=en-US ui_language=en-US region=US tz_windows=PST tz_iana=UTC inherit_children=1 audit=0 dns_mode=0",
        )
        .unwrap();
        bind_pid(&reg, 10, "p1", 0);
        reg.lock()
            .unwrap()
            .handle(&IpcMessage::ProcessCreated {
                pid: 10,
                child_pid: 11,
                image: "c.exe".into(),
            });
        let reply = reg.lock().unwrap().handle(&IpcMessage::GetProfile {
            pid: 11,
            profile_id: String::new(),
        });
        assert_eq!(reply.unwrap().name(), "PROFILE");
    }
}
