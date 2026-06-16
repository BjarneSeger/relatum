//! `relatum-domain` — the framework-free heart of the Relatum backend.
//!
//! This crate holds the **contracts** the rest of the system is built around. It
//! has zero dependencies on a web framework, a serialization format, or a
//! database driver, so business logic stays portable and testable in isolation.
//! Dependencies flow *inward*: `relatum-api` (transport) and `relatum-infra`
//! (storage / external systems) depend on this crate, never the other way round.
//!
//! Layers:
//! - [`models`] — plain domain entities and value objects.
//! - [`errors`] — [`DomainError`](errors::DomainError), the failure vocabulary
//!   of the domain. It carries no HTTP status; the API layer maps it to one.
//! - [`ports`] — traits describing what the domain *needs* from the outside
//!   world (e.g. persistence, the user directory, id generation). `relatum-infra`
//!   implements these.
//! - [`services`] — the use-cases the domain *offers* to the outside world, as
//!   concrete structs that consume [`ports`]. `relatum-api` calls these. Only the
//!   outbound ports are traits; the inbound services need no such abstraction.

pub mod errors;
pub mod models;
pub mod ports;
pub mod services;

// In-memory port doubles for tests. Public (behind the `testing` feature) so
// `relatum-api` can reuse them, and also compiled for this crate's own unit tests
// via `cfg(test)`. Excluded from release builds.
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use errors::DomainError;
