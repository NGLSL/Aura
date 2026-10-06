//! Explicit-bootstrap DoH prototype. No resolver, proxy, redirect, or pool.
mod budget;
mod error;
mod executor;
mod ffi;
mod offline_crl;
#[cfg(any(test, feature = "fixture-trust"))]
pub mod signed_ctl;
mod transport;
pub mod trust;
mod verification_scope;
pub use budget::{Budget, CancelCallback};
pub use error::Error;
pub use ffi::envbox_doh_query;

/// Standalone fixture callback instrumentation; absent from product/C ABI.
#[cfg(feature = "fixture-trust")]
pub fn fixture_cache_collection_active() -> bool {
    verification_scope::cache_collection_active()
}

#[cfg(feature = "fixture-trust")]
pub fn query_fixture(
    url: &str,
    literal_ip: &str,
    query: &[u8],
    budget: Budget,
    snapshot: trust::Snapshot,
) -> Result<Vec<u8>, Error> {
    transport::query(url, literal_ip, query, budget, Some(snapshot))
}
