//! [`SignatureStorage`] adapters.
//!
//! A user's signature lives in its own relational table, so two interchangeable
//! backends implement the same port:
//!
//! - [`InMemorySignatures`] — a self-contained `HashMap`; no external service, ideal
//!   for development, single-node deployments and tests.
//! - [`PostgresSignatureStore`] — backed by PostgreSQL via `sqlx`; compiled only
//!   when the `postgres` feature is enabled.
//!
//! [`SignatureStorage`]: relatum_domain::ports::signaturestorage::SignatureStorage

pub mod memory;
pub use memory::InMemorySignatures;

#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresSignatureStore;
