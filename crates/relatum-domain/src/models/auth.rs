//! Authentication value objects.

use crate::models::ids::UserId;

/// Credentials presented by a user when logging in.
///
/// Authentication is SSO-only: the instance manages no passwords (users and their
/// roles come from the periodic LDAP [sync](crate::services::sync::DirectorySync)).
/// A login therefore carries just the SSO access token the external provider
/// issued; the [`SSOProvider`](crate::ports::sso_connector::SSOProvider) validates
/// it and attests which user it belongs to.
#[derive(Debug, Clone)]
pub struct Credentials {
    /// The SSO access token issued by the external identity provider.
    pub token: String,
}

/// An opaque session token issued after a successful login.
///
/// Relatum uses session-based auth: a token is handed out on login, presented on
/// subsequent requests, and invalidated after a configurable lifetime (call
/// refresh to extend it). Token expiry metadata will live here later.
///
/// The `subject` binds the token to the user it authenticates, so a presented
/// token can be resolved back to its owner (see
/// [`Authenticator::authenticate`](crate::services::auth::Authenticator::authenticate)).
/// Only `value` is handed to the client; `subject` is server-side state.
#[derive(Debug, Clone)]
pub struct SessionToken {
    /// The opaque token value — the bearer credential presented by the client.
    pub value: String,
    /// The user this token authenticates.
    pub subject: UserId,
    // pub expires_at: ... — added when session lifetime is implemented.
}
