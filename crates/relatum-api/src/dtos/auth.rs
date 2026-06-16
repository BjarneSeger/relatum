//! Authentication DTOs.

use relatum_domain::models::auth::{Credentials, SessionToken};
use relatum_domain::ports::sso_connector::SsoMetadata;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Credentials posted to the login endpoint.
///
/// Authentication is SSO-only, so a login carries just the access token issued by
/// the external identity provider; the server validates it and resolves the user.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "0123456789abcdef")]
    pub token: String,
}

impl From<LoginRequest> for Credentials {
    fn from(value: LoginRequest) -> Self {
        Credentials { token: value.token }
    }
}

/// Body for redeeming a single-use SSO handoff code for a session token.
///
/// Posted to `POST /api/v1/auth/sso/exchange` by a browser frontend after it catches
/// the `code` the SSO callback redirected it with. The code is short-lived and
/// single-use, so it can travel through the browser without exposing a session token
/// in a URL; this back-channel call swaps it for the real session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SsoExchangeRequest {
    #[schema(example = "0123456789abcdef")]
    pub code: String,
}

/// A session token wrapper.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Token {
    token: String,
}

impl From<SessionToken> for Token {
    fn from(token: SessionToken) -> Self {
        Token { token: token.value }
    }
}

/// What the client needs to know to offer SSO login.
///
/// Returned by `GET /api/v1/auth/sso`: when `enabled`, the app shows the SSO button
/// and points the browser at `login_url` to begin the flow.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SsoInfo {
    /// Whether SSO login is available on this server.
    pub enabled: bool,
    /// Absolute URL of the SSO start endpoint, present when `enabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub login_url: Option<String>,
}

impl From<SsoMetadata> for SsoInfo {
    fn from(meta: SsoMetadata) -> Self {
        SsoInfo {
            enabled: meta.enabled,
            login_url: meta.login_url,
        }
    }
}

/// Response to a successful authentication.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthSuccess {
    #[serde(flatten)]
    pub token: Token,
}

impl From<SessionToken> for AuthSuccess {
    fn from(token: SessionToken) -> Self {
        AuthSuccess {
            token: token.into(),
        }
    }
}
