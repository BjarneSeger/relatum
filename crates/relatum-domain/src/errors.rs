//! The failure vocabulary of the domain.
//!
//! [`DomainError`] is intentionally framework-free: it knows nothing about HTTP
//! status codes, JSON, or any transport. The `relatum-api` layer owns the
//! mapping from each variant to a status code and wire body, so the domain never
//! depends on a web framework.

use thiserror::Error;

/// Everything a domain operation can fail with.
///
/// Keep variants coarse and transport-agnostic. The API layer translates these
/// into HTTP responses (see `relatum_api::ApiError`).
#[derive(Debug, Error)]
pub enum DomainError {
    /// The requested resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The input failed validation / was semantically malformed.
    #[error("invalid input: {0}")]
    Invalid(String),

    /// Authentication is required or the supplied credentials/token are invalid.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// The caller is authenticated but not permitted to perform this action.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// The request conflicts with existing state, e.g. a uniqueness rule (a
    /// trainee already has a report for the week).
    #[error("conflict: {0}")]
    Conflict(String),

    /// A downstream dependency (storage, the user directory, the session store, …) failed.
    #[error("backend error: {0}")]
    Backend(String),

    /// An unexpected internal error.
    #[error("internal error: {0}")]
    Internal(String),
}
