//! Trusted service protocol and Windows SCM host. No installation or driver
//! loading is performed by this crate. Container qualification remains closed.
#[cfg(windows)]
mod bundle;
pub mod protocol;
#[cfg(windows)]
pub mod prototype;
#[cfg(windows)]
mod snapshot;
#[cfg(windows)]
pub mod windows_service;
