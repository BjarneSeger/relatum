//! Key-value [`EphemeralStore`] adapters.
//!
//! Short-lived, single-use state (the OIDC flow's pending logins and handoff codes)
//! lives in a key-value store with a TTL, like sessions do. Two interchangeable
//! backends implement the same port:
//!
//! - [`InMemoryTtlEphemeralStore`] — a self-contained `HashMap` with per-entry TTL;
//!   no external service, ideal for development and single-node deployments.
//! - [`ValkeyEphemeralStore`] — backed by Valkey/Redis with native key expiry and an
//!   atomic `GETDEL` take, so the state is shared across replicas; compiled only when
//!   the `valkey` feature is enabled.
//!
//! [`EphemeralStore`]: relatum_domain::ports::ephemeral::EphemeralStore

pub mod memory;
pub use memory::InMemoryTtlEphemeralStore;

#[cfg(feature = "valkey")]
pub mod valkey;
#[cfg(feature = "valkey")]
pub use valkey::ValkeyEphemeralStore;
