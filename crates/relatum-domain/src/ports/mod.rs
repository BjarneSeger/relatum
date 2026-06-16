//! Ports: the contracts the domain *needs* from the outside world.
//!
//! A port is a trait the domain depends on but does not implement — persistence,
//! external services, clocks, and so on. `relatum-infra` provides the concrete
//! implementations. This is the seam that keeps business logic independent of any
//! particular database or technology.

pub mod directory;
pub mod ephemeral;
pub mod ids;
pub mod reportstorage;
pub mod session;
pub mod sso_connector;
pub mod status;
pub mod userstorage;
