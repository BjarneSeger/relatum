//! Persistence contract for session tokens.

use std::future::Future;

use crate::errors::DomainError;
use crate::models::auth::SessionToken;
use crate::ports::status::StatusBackend;

/// Stores and retrieves issued session tokens.
///
/// Implemented in `relatum-infra` (e.g. backed by Valkey). The auth service
/// depends on this port to persist sessions without knowing the storage details.
///
/// The methods return `Send` futures (and `StatusBackend` makes implementors
/// `Send + Sync`) so the auth service can sit behind the API's shared axum state.
pub trait SessionRepository: StatusBackend {
    /// Persist a freshly issued token.
    fn store(&self, token: &SessionToken) -> impl Future<Output = Result<(), DomainError>> + Send;

    /// Look up a token by its value, returning `None` if it is unknown/expired.
    fn lookup(
        &self,
        value: &str,
    ) -> impl Future<Output = Result<Option<SessionToken>, DomainError>> + Send;

    /// Invalidate a token so it can no longer be used.
    fn revoke(&self, value: &str) -> impl Future<Output = Result<(), DomainError>> + Send;
}
