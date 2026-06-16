//! Shared application state.

use axum_extra::extract::CookieJar;
use relatum_client::Client;

use crate::error::WebError;
use crate::session;

/// Everything a handler needs that is fixed for the process lifetime. Cheap to
/// [`Clone`] (a few strings), so it sits behind axum's shared state.
#[derive(Clone)]
pub struct WebState {
    /// Base URL of the relatum API (no trailing slash).
    pub api_url: String,
    /// The web app's own externally-reachable base URL (no trailing slash).
    pub public_url: String,
    /// Whether the session cookie is marked `Secure` (set when `public_url` is https).
    pub secure: bool,
    /// Departments offered in the admin dropdown.
    pub departments: Vec<String>,
}

impl WebState {
    /// A client with no session — only the public endpoints (login, sso info) are
    /// meaningful on it.
    pub fn anon_client(&self) -> Client {
        Client::new(self.api_url.clone())
    }

    /// A client bound to the caller's session token, or [`WebError::NeedsLogin`] when
    /// there is no session cookie. The token's *validity* is only learned when the
    /// API answers `401`, which [`From<ClientError>`](WebError) also maps to
    /// `NeedsLogin`, so an expired session and a missing one funnel to the same place.
    pub fn authed(&self, jar: &CookieJar) -> Result<Client, WebError> {
        match session::token(jar) {
            Some(token) => Ok(Client::with_token(self.api_url.clone(), Some(token))),
            None => Err(WebError::NeedsLogin),
        }
    }
}
