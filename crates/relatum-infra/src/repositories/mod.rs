//! Concrete implementations of the domain's repository ports.
//!
//! Each module here provides a struct that implements a trait from
//! `relatum_domain::ports`, translating between storage rows and domain models.
//! Every port follows the same shape: a self-contained in-memory adapter for
//! development and tests, alongside a feature-gated adapter for the production
//! backing service.

pub mod ephemeral;
pub mod report;
pub mod session;
pub mod user;
