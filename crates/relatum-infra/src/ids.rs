//! UUID-backed [`IdGenerator`] implementation.

use relatum_domain::models::ids::ReportId;
use relatum_domain::ports::ids::IdGenerator;
use uuid::Uuid;

/// Generates report ids and session-token values from random (v4) UUIDs.
///
/// Holds no state, so it is cheap to clone and share across handlers.
#[derive(Debug, Clone, Default)]
pub struct UuidIdGenerator;

impl IdGenerator for UuidIdGenerator {
    fn report_id(&self) -> ReportId {
        ReportId::new(Uuid::new_v4().to_string())
    }

    fn session_token(&self) -> String {
        // Two v4 UUIDs (~244 bits of entropy), hyphen-free, for an opaque bearer
        // token that is infeasible to guess.
        format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
    }
}
