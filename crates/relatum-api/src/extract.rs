//! Request extractors that bridge HTTP credentials to the domain.
//!
//! - [`BearerToken`] pulls the raw token out of the `Authorization` header.
//! - [`CurrentUser`] goes one step further and resolves that token to the
//!   authenticated [`User`] via
//!   [`Authenticator::authenticate`](relatum_domain::services::auth::Authenticator::authenticate),
//!   so handlers receive the acting user directly. A missing or invalid token is
//!   rejected with `401` before the handler runs.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use relatum_domain::models::users::User;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;

use crate::error::ApiError;
use crate::state::AppState;

/// The raw bearer token presented in the `Authorization: Bearer <token>` header.
///
/// State-agnostic, so any handler (e.g. logout/refresh, which only need the token
/// value, not the resolved user) can ask for it.
pub struct BearerToken(pub String);

impl<St: Send + Sync> FromRequestParts<St> for BearerToken {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &St) -> Result<Self, Self::Rejection> {
        Ok(BearerToken(bearer_token(parts)?.to_owned()))
    }
}

/// The authenticated user behind a request's bearer token.
pub struct CurrentUser(pub User);

impl<U, S, I, P, R> FromRequestParts<AppState<U, S, I, P, R>> for CurrentUser
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState<U, S, I, P, R>,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)?.to_owned();
        let user = state.auth.authenticate(&token).await?;
        Ok(CurrentUser(user))
    }
}

/// Pull `<token>` out of an `Authorization: Bearer <token>` header, or fail with
/// `401`.
fn bearer_token(parts: &Parts) -> Result<&str, ApiError> {
    let header = parts
        .headers
        .get(AUTHORIZATION)
        .ok_or_else(|| ApiError::Unauthorized("missing Authorization header".into()))?;
    let value = header
        .to_str()
        .map_err(|_| ApiError::Unauthorized("malformed Authorization header".into()))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError::Unauthorized("expected a Bearer token".into()))
}
