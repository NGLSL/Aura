//! Launch pipeline: command resolution, Environment Block, Job Object, process create.
//! Runtime injection is ticket 04; this module owns the launch path first.

pub mod command;
pub mod environment;
pub mod injection;
pub mod instance;
pub mod job;
pub mod launcher;

pub use command::{resolve_command, ResolvedCommand};
pub use environment::{build_environment_block, encode_environment_block};
pub use injection::{
    is_elevation_integrity, map_create_process_error, pe_arch, resolve_runtime_dll,
    resolve_runtime_dll_for_target, InjectError, PeArch,
};
pub use instance::{InstanceError, InstanceHandle, InstanceManager, RunTarget};
pub use job::InstanceJob;
pub use launcher::{format_args, launch, parse_args, LaunchError, LaunchRequest, LaunchedProcess};
