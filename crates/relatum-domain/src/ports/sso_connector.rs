//! Contracts for contacting a SSO provider like Keycloak or Authentik.

use std::future::Future;

use crate::{DomainError, models::ids::UserId, ports::status::StatusBackend};

/// The identity an [`SSOProvider`] attests for a valid token.
///
/// The provider is the *authentication* authority only: it answers "which user
/// does this token belong to". It no longer maps roles — a user's role and
/// department come from the periodic LDAP
/// [sync](crate::services::sync::DirectorySync), not from the login token. The
/// attested user must already exist locally (the sync provisions them); an SSO
/// login for an unknown subject is rejected.
#[derive(Debug, Clone)]
pub struct SsoIdentity {
    /// Stable local identity the token's subject maps to.
    pub id: UserId,
}

/// What a client needs to know to start an SSO login.
///
/// Advertised over the API (`GET /api/v1/auth/sso`) so a client (the web frontend or
/// CLI) can decide whether to show the SSO button and where to point the browser. With SSO
/// disabled, `enabled` is `false` and `login_url` is `None`.
#[derive(Debug, Clone)]
pub struct SsoMetadata {
    /// Whether an external IdP is configured and SSO login is possible.
    pub enabled: bool,
    /// Absolute URL of the server's SSO start endpoint, when enabled.
    pub login_url: Option<String>,
}

/// The outcome of completing the browser leg of an SSO login.
///
/// Pairs the access token obtained from the IdP (to be exchanged for a relatum
/// session) with the client's loopback redirect the browser should return to.
#[derive(Debug, Clone)]
pub struct SsoCompletion {
    /// The OIDC access token the provider issued for the authenticated user.
    pub access_token: String,
    /// The app's loopback URL to redirect the browser back to.
    pub app_redirect: String,
}

/// Allows contacting some kind of Single-Sign-On provider, asking whether the token
/// the user submitted is actually valid — and, for providers that can drive the
/// browser login, running the OAuth authorization-code flow on the client's behalf.
///
/// The flow methods ([`metadata`](SSOProvider::metadata),
/// [`begin`](SSOProvider::begin), [`complete`](SSOProvider::complete)) default to
/// "SSO unavailable", so a provider that only *validates* tokens (or none at all)
/// need implement nothing extra.
pub trait SSOProvider: StatusBackend {
    /// Checks whether the given token is valid, returning the attested identity if
    /// it is.
    fn check_token(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<SsoIdentity>, DomainError>> + Send;

    /// Whether SSO login is available, and where the browser flow starts.
    fn metadata(&self) -> SsoMetadata {
        SsoMetadata {
            enabled: false,
            login_url: None,
        }
    }

    /// Begin the browser login: stash the per-attempt state bound to `app_redirect`
    /// (the client's loopback redirect URL) and return the IdP authorization URL to send
    /// the browser to. Defaults to rejecting, since the base provider drives no flow.
    fn begin(
        &self,
        _app_redirect: &str,
    ) -> impl Future<Output = Result<String, DomainError>> + Send {
        async { Err(DomainError::Unauthorized("SSO login is not available".into())) }
    }

    /// Complete the browser login: exchange the authorization `code` (matched to a
    /// pending `state`) for an access token, returning it alongside the app redirect
    /// stored in [`begin`](SSOProvider::begin). Defaults to rejecting.
    fn complete(
        &self,
        _code: &str,
        _state: &str,
    ) -> impl Future<Output = Result<SsoCompletion, DomainError>> + Send {
        async { Err(DomainError::Unauthorized("SSO login is not available".into())) }
    }

    /// Stash an obtained access token under a fresh **single-use handoff code**,
    /// returning the code.
    ///
    /// This exists so a browser SSO flow never has to carry the access (or session)
    /// token in a redirect URL: only the opaque, short-lived, single-use code travels
    /// through the browser, and the token is fetched back-channel by
    /// [`redeem_handoff`](SSOProvider::redeem_handoff). Defaults to rejecting, since a
    /// provider that drives no browser flow needs no handoff store.
    fn stash_handoff(
        &self,
        _access_token: String,
    ) -> impl Future<Output = Result<String, DomainError>> + Send {
        async { Err(DomainError::Unauthorized("SSO login is not available".into())) }
    }

    /// Redeem a handoff `code` for the access token it was issued for, **consuming it**
    /// (single-use). An unknown, already-used, or expired code is rejected with
    /// [`DomainError::Unauthorized`]. Defaults to rejecting.
    fn redeem_handoff(
        &self,
        _code: &str,
    ) -> impl Future<Output = Result<String, DomainError>> + Send {
        async { Err(DomainError::Unauthorized("invalid or expired SSO handoff".into())) }
    }
}
