//! Launch pipeline: command resolution, Environment Block, Job Object, process create.
//! Control plane (packaged-v1): ActivationBackend / RuntimeAttacher / EnvironmentSession.

pub mod activation;
pub mod attach;
pub mod capability;
pub mod command;
pub mod environment;
pub mod injection;
pub mod instance;
pub mod ipc;
pub mod ipc_server;
pub mod job;
pub mod launcher;
pub mod package_discovery;
pub mod process_tracker;
pub mod recovery;
pub mod session;

pub use activation::{
    backend_for, ActivateError, ActivatedTarget, ActivationBackend, ActivationRequest,
    PackagedActivationBackend, Win32ActivationBackend,
};
pub use attach::{select_strategy, AttachError, AttachedRuntime, RuntimeAttacher, RuntimeInjector};
pub use capability::{capabilities_after_probe, probe_pid, win32_capabilities, ProcessProbe};
pub use command::{resolve_command, ResolvedCommand};
pub use environment::{build_environment_block, encode_environment_block};
pub use injection::{
    is_elevation_integrity, map_create_process_error, pe_arch, resolve_runtime_dll,
    resolve_runtime_dll_for_target, stage_runtime_dll, InjectError, PeArch,
};
pub use instance::{InstanceError, InstanceHandle, InstanceManager, RunTarget};
pub use ipc::{
    message_to_profile, profile_to_message, round_trip, FakeBroker, IpcError, IpcMessage,
    SessionTable, DEFAULT_PIPE_NAME,
};
pub use ipc_server::{session_pipe_name, HostBroker, SharedTable};
pub use job::InstanceJob;
pub use launcher::{
    format_args, launch, open_process_handle, parse_args, resume_activated, spawn_for_activation,
    LaunchError, LaunchRequest, LaunchedProcess,
};
pub use package_discovery::{
    discover_packages, extract_aumid, is_windows_apps_path, launch_target_from_user_path,
    normalize_launch_target, parse_aumid, resolve_package_identity, PackageAppInfo,
};
pub use process_tracker::{ProcessTracker, TrackMode};
pub use recovery::{
    read_runtime_capabilities, request_runtime_reconnect, validate_runtime_for_profile,
    RecoveryError, RuntimeCapabilities,
};
pub use session::{
    belongs, register_child, start_session, start_session_gated, start_session_in_named_job,
    start_session_in_named_job_gated, start_session_in_new_console, terminal_cancel_marker_path,
    terminal_root_marker_path, SessionError, SessionHandle, SessionStartRequest,
};
