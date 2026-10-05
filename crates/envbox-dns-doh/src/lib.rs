//! Explicit-bootstrap DoH prototype. No resolver, proxy, redirect, or pool.
mod budget;
mod error;
mod executor;
mod ffi;
mod transport;
pub mod trust;
pub use budget::{Budget, CancelCallback};
pub use error::Error;
pub use ffi::envbox_doh_query;

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
