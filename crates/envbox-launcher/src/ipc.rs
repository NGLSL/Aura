//! Runtime ↔ Host IPC protocol (packaged-v1 ticket 40).
//!
//! Wire format matches `runtime/src/ipc_bootstrap.h`:
//! one line per message, UTF-8, `MSG_NAME key=value ...`, values are bare
//! tokens or double-quoted strings (`\\` `\"` `\n` `\r` `\t` escapes).
//! Lists use repeated keys (`dns_server=`, `registry_path=`, `environment=`).
//!
//! Public message names (stable, case-sensitive):
//! HELLO / GET_PROFILE / PROFILE / RUNTIME_READY / HOOK_ERROR /
//! PROCESS_CREATED / PROCESS_EXITED.
//!
//! Win32 keeps ENVBOX_* structured values as fallback; IPC/Broker is preferred
//! for all roots (including packaged, which have no Environment Block).

mod profile;
mod session;
mod testing;
mod wire;

pub use profile::{
    message_to_profile, profile_to_message, profile_to_message_with_flags,
    RUNTIME_ENVIRONMENT_ENTRY_MAX_BYTES, RUNTIME_ENVIRONMENT_MAX,
};
pub use session::{ObservedRuntimeIdentity, SessionTable};
pub use testing::{round_trip, FakeBroker};
pub use wire::{
    IpcError, IpcMessage, RuntimeIdentity, DEFAULT_PIPE_NAME, IPC_IDENTITY_MAX_LINE_BYTES,
    IPC_MAX_LINE_BYTES, RUNTIME_IDENTITY_PROTOCOL,
};

#[cfg(test)]
mod tests;
