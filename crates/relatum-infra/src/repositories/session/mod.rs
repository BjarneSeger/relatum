//! Key-value [`SessionRepository`] adapters.
//!
//! Sessions are ephemeral, so they live in a key-value store rather than the
//! relational tables used for users and reports. Two interchangeable backends
//! implement the same port:
//!
//! - [`InMemoryTtlSessionStore`] — a self-contained `HashMap` with per-entry TTL;
//!   no external service, ideal for development and single-node deployments.
//! - [`ValkeySessionStore`] — backed by Valkey/Redis with native key expiry;
//!   compiled only when the `valkey` feature is enabled.
//!
//! [`SessionRepository`]: relatum_domain::ports::session::SessionRepository

pub mod memory;
pub use memory::InMemoryTtlSessionStore;

#[cfg(feature = "valkey")]
pub mod valkey;
#[cfg(feature = "valkey")]
pub use valkey::ValkeySessionStore;
