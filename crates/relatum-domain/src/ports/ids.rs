//! Generation of fresh identifiers and opaque tokens.
//!
//! Minting a [`ReportId`] or a session-token value needs a source of
//! uniqueness/randomness (a UUID generator, a CSPRNG, …) that the domain must not
//! depend on directly — that would drag `uuid`/`rand` into the otherwise
//! framework-free core. This port keeps the seam: the services ask for a fresh
//! value, `relatum-infra` decides how it is produced.

use crate::models::ids::ReportId;

/// Source of fresh identifiers and opaque token values.
///
/// Implementations must return a distinct value on every call. `session_token`
/// in particular must be unpredictable (cryptographically random), since it is
/// the bearer credential handed out on login. `Send + Sync` so it can live inside
/// the domain services the API layer shares across threads.
pub trait IdGenerator: Send + Sync {
    /// A fresh, unique report identifier.
    fn report_id(&self) -> ReportId;

    /// A fresh, high-entropy opaque session-token value.
    fn session_token(&self) -> String;
}
