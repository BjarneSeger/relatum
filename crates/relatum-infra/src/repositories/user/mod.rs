//! [`UserStorage`] adapters.
//!
//! Users live in a relational table, so two interchangeable backends implement
//! the same port:
//!
//! - [`InMemoryUsers`] — a self-contained `HashMap`; no external service, ideal
//!   for development, single-node deployments and tests.
//! - [`PostgresUserStore`] — backed by PostgreSQL via `sqlx`; compiled only when
//!   the `postgres` feature is enabled.
//!
//! [`UserStorage`]: relatum_domain::ports::userstorage::UserStorage

pub mod memory;
pub use memory::InMemoryUsers;

#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresUserStore;
