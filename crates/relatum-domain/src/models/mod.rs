//! Domain entities and value objects.
//!
//! These are plain Rust types with no serialization or framework derives. The
//! API layer defines its own DTOs and converts to/from these at the boundary,
//! keeping the wire format decoupled from the internal representation.

pub mod auth;
pub mod department;
pub mod ids;
pub mod meta;
pub mod report;
pub mod signature;
pub mod users;
pub mod week;
