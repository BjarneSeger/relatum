//! `relatum-infra` — concrete implementations of the domain's [ports].
//!
//! This is the outer layer: it adapts external technology (databases, an LDAP
//! directory, an OIDC provider, …) to the contracts defined in `relatum-domain`. It
//! depends on the domain, never the other way round.
//!
//! Each storage port ships two interchangeable adapters: a self-contained
//! in-memory one for development and tests, and a feature-gated one backed by a
//! production service (PostgreSQL behind `postgres`, Valkey behind `valkey`).
//! See [`repositories`].
//!
//! [ports]: relatum_domain::ports

pub mod db;
#[cfg(feature = "ldap")]
pub mod directory;
pub mod ids;
pub mod repositories;
pub mod sso;
