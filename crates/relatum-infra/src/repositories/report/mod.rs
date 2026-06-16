//! [`ReportStorage`] adapters.
//!
//! Reports live in a relational table, so two interchangeable backends implement
//! the same port:
//!
//! - [`InMemoryReports`] — a self-contained `HashMap`; no external service, ideal
//!   for development, single-node deployments and tests.
//! - [`PostgresReportStore`] — backed by PostgreSQL via `sqlx`; compiled only
//!   when the `postgres` feature is enabled.
//!
//! [`ReportStorage`]: relatum_domain::ports::reportstorage::ReportStorage

pub mod memory;
pub use memory::InMemoryReports;

#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresReportStore;
