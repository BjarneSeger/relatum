//! A no-op [`SSOProvider`] for deployments without an external IdP.
//!
//! The domain's [`Authenticator`] requires an `SSOProvider` to satisfy the `P`
//! port. [`DisabledSso`] fills that slot when no IdP is configured: it attests no
//! token, so every login is rejected as `Unauthorized` (authentication is SSO-only
//! now). The [`OidcSso`](super::OidcSso) adapter replaces it once an OpenID Connect
//! provider is wired up.
//!
//! [`Authenticator`]: relatum_domain::services::auth::Authenticator
//! [`SSOProvider`]: relatum_domain::ports::sso_connector::SSOProvider

use relatum_domain::errors::DomainError;
use relatum_domain::ports::sso_connector::{SSOProvider, SsoIdentity};
use relatum_domain::ports::status::{PortStatus, StatusBackend};

/// An [`SSOProvider`] that never validates a token.
///
/// Holds no state, so it is cheap to clone and share across handlers.
#[derive(Debug, Clone, Default)]
pub struct DisabledSso;

impl SSOProvider for DisabledSso {
    /// Always attests no identity, so every SSO login fails as `Unauthorized`.
    async fn check_token(&self, _token: &str) -> Result<Option<SsoIdentity>, DomainError> {
        Ok(None)
    }
}

impl StatusBackend for DisabledSso {
    /// Reports as unconnected — there is no upstream provider to reach.
    async fn get_status(&self) -> PortStatus {
        PortStatus::NotConnected
    }
}
