//! An OpenID Connect [`SSOProvider`] that validates tokens against a provider's
//! userinfo endpoint.
//!
//! The submitted token is treated as an OIDC access token: the adapter calls the
//! provider's userinfo endpoint with `Authorization: Bearer <token>` and maps the
//! `sub` claim onto a [`SsoIdentity`]. The provider is the *authentication*
//! authority only — it answers "which user is this token" and nothing more. A
//! user's role and department come from the periodic LDAP sync, not the token, so
//! the group/team claim mapping that used to live here is gone.
//!
//! Compiled only when the `oidc` feature is enabled.
//!
//! [`SSOProvider`]: relatum_domain::ports::sso_connector::SSOProvider

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::UserId;
use relatum_domain::ports::ephemeral::EphemeralStore;
use relatum_domain::ports::sso_connector::{SSOProvider, SsoCompletion, SsoIdentity, SsoMetadata};
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use reqwest::{Client, StatusCode, Url};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// How long a started login may sit unfinished before its pending state expires.
const PENDING_TTL: Duration = Duration::from_secs(300);

/// How long a completed login's handoff code stays redeemable. Short: the app
/// redeems it back-channel immediately after the browser lands on its callback.
const HANDOFF_TTL: Duration = Duration::from_secs(60);

/// The authorization-code-flow configuration: the IdP endpoints and the confidential
/// client credentials the server holds, plus its own public base URL for building the
/// callback the IdP redirects to.
#[derive(Debug, Clone)]
pub struct OidcFlow {
    /// The IdP authorization endpoint the browser is sent to.
    pub authorize_url: String,
    /// The IdP token endpoint the authorization code is exchanged at.
    pub token_url: String,
    /// The confidential client's id, registered with the IdP.
    pub client_id: String,
    /// The confidential client's secret.
    pub client_secret: String,
    /// Space-separated scopes to request (e.g. `openid profile groups`).
    pub scopes: String,
    /// The server's externally reachable base URL, used to build the redirect URI
    /// `{public_url}/api/v1/auth/sso/callback` registered with the IdP.
    pub public_url: String,
    /// Origins (`scheme://host[:port]`) the browser may be returned to once login
    /// completes — typically the web frontend's origin. The login result (the
    /// single-use handoff code) is only ever delivered to one of these or to a
    /// loopback address (the native CLI); any other `redirect_uri` is rejected, so a
    /// caller cannot point the flow at a site they control. See [`redirect_allowed`].
    ///
    /// Only the **origin** (scheme, host, port) of each entry matters — any path is
    /// ignored. Each entry is parsed and validated once at startup (see
    /// [`OidcSso::new`]); a malformed entry fails the boot.
    pub allowed_redirects: Vec<String>,
}

/// An [`SSOProvider`] backed by an OIDC userinfo endpoint.
///
/// Holds a [`reqwest::Client`] (cheap to clone, internally pooled), the
/// authorization-code-flow configuration, and an [`EphemeralStore`] for the flow's
/// short-lived state, so the adapter is `Clone + Send + Sync` and fits behind shared
/// async state. Backing the flow state with a shared store (rather than a per-process
/// map) is what lets a login started on one replica complete on another.
#[derive(Debug, Clone)]
pub struct OidcSso<E> {
    http: Client,
    /// The provider's userinfo endpoint, queried with the bearer token.
    userinfo_url: String,
    /// The authorization-code-flow configuration (endpoints + client credentials).
    flow: OidcFlow,
    /// The `flow.allowed_redirects` entries parsed into URLs once, at construction, so
    /// each login start compares against ready origins instead of re-parsing the list.
    /// A malformed entry is rejected at startup (see [`OidcSso::new`]).
    allowed_origins: Vec<Url>,
    /// Short-lived flow state with native TTL: pending logins keyed `pending:<state>`
    /// (the PKCE verifier + app redirect, written in [`begin`] and taken in
    /// [`complete`]) and single-use handoff codes keyed `handoff:<code>` (written in
    /// [`stash_handoff`], taken in [`redeem_handoff`]). When this is a shared store,
    /// any replica can complete a login another replica began.
    ///
    /// [`begin`]: SSOProvider::begin
    /// [`complete`]: SSOProvider::complete
    /// [`stash_handoff`]: SSOProvider::stash_handoff
    /// [`redeem_handoff`]: SSOProvider::redeem_handoff
    ephemeral: E,
}

impl<E> OidcSso<E> {
    /// Build an adapter for the given userinfo endpoint and authorization-code-flow
    /// configuration, storing the flow's short-lived state in `ephemeral`.
    pub fn new(userinfo_url: String, flow: OidcFlow, ephemeral: E) -> Result<Self, DomainError> {
        let http = Client::builder()
            .build()
            .map_err(|e| DomainError::Backend(format!("building OIDC http client failed: {e}")))?;
        // Parse the redirect allowlist once, here at startup, so a malformed entry fails
        // the boot loudly instead of being silently skipped on every login. Only each
        // entry's origin is ever compared; its path is irrelevant (see `redirect_allowed`).
        let allowed_origins = flow
            .allowed_redirects
            .iter()
            .map(|entry| {
                Url::parse(entry).map_err(|e| {
                    DomainError::Backend(format!(
                        "invalid sso.allowed_redirects entry '{entry}': {e}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            http,
            userinfo_url,
            flow,
            allowed_origins,
            ephemeral,
        })
    }

    /// The callback URL the IdP redirects to after authentication — must match what
    /// is registered with the IdP for the client.
    fn callback_url(&self) -> String {
        format!(
            "{}/api/v1/auth/sso/callback",
            self.flow.public_url.trim_end_matches('/')
        )
    }

    /// Map a userinfo claim set onto a [`SsoIdentity`].
    ///
    /// Pure (no I/O), so the claim → identity rule is unit-testable without a live
    /// provider. The provider only authenticates, so all that is read is the `sub`
    /// claim — the stable subject the token belongs to. A token without a `sub` is
    /// rejected as `Unauthorized`, matching the contract that an invalid token
    /// yields no identity. Role and department come from the LDAP sync, not here.
    fn map_claims(&self, claims: &HashMap<String, Value>) -> Result<SsoIdentity, DomainError> {
        let sub = claims
            .get("sub")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DomainError::Unauthorized("SSO token rejected: missing sub claim".into())
            })?;

        Ok(SsoIdentity {
            id: UserId::new(sub),
        })
    }
}

impl<E: EphemeralStore> SSOProvider for OidcSso<E> {
    // `token` is the user's OIDC access token — skipped from the span fields.
    #[tracing::instrument(skip(self, token), level = "debug")]
    async fn check_token(&self, token: &str) -> Result<Option<SsoIdentity>, DomainError> {
        let response = self
            .http
            .get(&self.userinfo_url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "OIDC userinfo request failed");
                DomainError::Backend(format!("OIDC userinfo request failed: {e}"))
            })?;

        // An unauthenticated/forbidden response means the token is simply invalid,
        // not that the provider is broken — attest no identity.
        if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            tracing::debug!("sso token rejected by provider");
            return Ok(None);
        }
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "OIDC userinfo returned non-success");
            return Err(DomainError::Backend(format!(
                "OIDC userinfo returned {}",
                response.status()
            )));
        }

        let claims: HashMap<String, Value> = response
            .json()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "decoding OIDC userinfo failed");
                DomainError::Backend(format!("decoding OIDC userinfo failed: {e}"))
            })?;

        self.map_claims(&claims).map(Some)
    }

    fn metadata(&self) -> SsoMetadata {
        SsoMetadata {
            enabled: true,
            login_url: Some(format!(
                "{}/api/v1/auth/sso/start",
                self.flow.public_url.trim_end_matches('/')
            )),
        }
    }

    // `app_redirect` is a destination URL (not secret); the PKCE verifier/state/challenge
    // generated below are, and are never logged.
    #[tracing::instrument(skip(self, app_redirect), fields(redirect = %app_redirect), level = "debug")]
    async fn begin(&self, app_redirect: &str) -> Result<String, DomainError> {
        // Only ever return the browser to an allowlisted (or loopback) destination.
        // Without this, an attacker could start a login with a `redirect_uri` they
        // control; after the victim authenticates, the callback would hand *their*
        // single-use handoff code to the attacker's site, which the attacker redeems
        // back-channel for the victim's session (open redirect → account takeover).
        if !redirect_allowed(app_redirect, &self.allowed_origins) {
            tracing::warn!(redirect = %app_redirect, "sso redirect_uri rejected (not an allowed destination)");
            return Err(DomainError::Forbidden(
                "redirect_uri is not an allowed destination".into(),
            ));
        }

        let state = random_token();
        let verifier = random_token();
        let challenge = pkce_challenge(&verifier);

        // Stash the PKCE verifier and the app redirect under the `state`, with a TTL,
        // so `complete` can recover them when the IdP redirects the browser back —
        // possibly to a different replica when the store is shared.
        let pending = json!({ "verifier": verifier, "app_redirect": app_redirect }).to_string();
        self.ephemeral
            .put(&pending_key(&state), &pending, PENDING_TTL)
            .await?;

        let redirect = self.callback_url();
        let url = Url::parse_with_params(
            &self.flow.authorize_url,
            &[
                ("response_type", "code"),
                ("client_id", self.flow.client_id.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("scope", self.flow.scopes.as_str()),
                ("state", state.as_str()),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ],
        )
        .map_err(|e| {
            tracing::error!(error = %e, "building OIDC authorize URL failed");
            DomainError::Backend(format!("building authorize URL failed: {e}"))
        })?;
        Ok(url.into())
    }

    // `code` (authorization code) and `state` (CSRF token) are both secret — skipped.
    #[tracing::instrument(skip(self, code, state), level = "debug")]
    async fn complete(&self, code: &str, state: &str) -> Result<SsoCompletion, DomainError> {
        // Take (single-use) the pending entry stashed by `begin`. An unknown/expired
        // `state` — or one already consumed — yields `None`.
        let raw = self
            .ephemeral
            .take(&pending_key(state))
            .await?
            .ok_or_else(|| {
                tracing::debug!("sso state unknown or expired");
                DomainError::Unauthorized("unknown or expired SSO state".into())
            })?;
        let pending: Value = serde_json::from_str(&raw)
            .map_err(|e| DomainError::Backend(format!("decoding pending SSO state failed: {e}")))?;
        let verifier = pending
            .get("verifier")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::Backend("pending SSO state missing verifier".into()))?;
        let app_redirect = pending
            .get("app_redirect")
            .and_then(Value::as_str)
            .ok_or_else(|| DomainError::Backend("pending SSO state missing app_redirect".into()))?
            .to_owned();

        let redirect = self.callback_url();
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect.as_str()),
            ("client_id", self.flow.client_id.as_str()),
            ("client_secret", self.flow.client_secret.as_str()),
            ("code_verifier", verifier),
        ];
        let response = self
            .http
            .post(&self.flow.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "OIDC token request failed");
                DomainError::Backend(format!("OIDC token request failed: {e}"))
            })?;

        // A non-success here means the code/verifier was rejected: a failed login,
        // not a broken provider.
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "OIDC token endpoint rejected the exchange");
            return Err(DomainError::Unauthorized(format!(
                "OIDC token endpoint returned {}",
                response.status()
            )));
        }

        let body: HashMap<String, Value> = response
            .json()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "decoding OIDC token response failed");
                DomainError::Backend(format!("decoding OIDC token response failed: {e}"))
            })?;
        let access_token = body
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DomainError::Backend("OIDC token response missing access_token".into())
            })?
            .to_owned();

        Ok(SsoCompletion {
            access_token,
            app_redirect,
        })
    }

    // `access_token` is secret — skipped from the span fields.
    #[tracing::instrument(skip(self, access_token), level = "debug")]
    async fn stash_handoff(&self, access_token: String) -> Result<String, DomainError> {
        let code = random_token();
        self.ephemeral
            .put(&handoff_key(&code), &access_token, HANDOFF_TTL)
            .await?;
        Ok(code)
    }

    // `code` is the single-use handoff secret — skipped from the span fields.
    #[tracing::instrument(skip(self, code), level = "debug")]
    async fn redeem_handoff(&self, code: &str) -> Result<String, DomainError> {
        // Take is single-use: the store removes the code as it returns it, so a code
        // cannot be redeemed twice even if two replicas race on it.
        self.ephemeral
            .take(&handoff_key(code))
            .await?
            .ok_or_else(|| {
                tracing::debug!("sso handoff invalid or expired");
                DomainError::Unauthorized("invalid or expired SSO handoff".into())
            })
    }
}

/// A random, URL-safe token (64 hex chars) for use as a PKCE verifier or `state`.
/// Two v4 UUIDs of entropy, well within PKCE's 43–128 unreserved-char range.
fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// The S256 PKCE challenge for `verifier`: base64url(sha256(verifier)), unpadded.
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Ephemeral-store key for a pending login, namespaced by the `state` handed to the IdP.
fn pending_key(state: &str) -> String {
    format!("pending:{state}")
}

/// Ephemeral-store key for a stashed handoff, namespaced by its single-use `code`.
fn handoff_key(code: &str) -> String {
    format!("handoff:{code}")
}

/// Whether the browser may be returned to `app_redirect` once login completes.
///
/// Accepts two kinds of destination and nothing else:
/// - a **loopback** URL (`http://127.0.0.1|[::1]|localhost:<port>/…`) — the native
///   CLI's RFC 8252 callback, safe because it resolves to the user's own machine, so
///   an attacker cannot receive a code delivered there;
/// - any URL whose **origin** (scheme + host + effective port) exactly matches one of
///   the configured `allowed` origins — typically the web frontend's origin.
///
/// `allowed` holds the configured redirects already parsed to URLs (done once at
/// construction). Only each entry's **origin** is significant: any path it carries is
/// ignored, so `https://app.example.com` and `https://app.example.com/app` are
/// equivalent. A URL that fails to parse, or matches neither kind, is rejected. Origin
/// matching (not a substring/prefix check) is deliberate: it stops look-alikes such as
/// `https://relatum.example.evil.com` and `userinfo@`/path tricks.
fn redirect_allowed(app_redirect: &str, allowed: &[Url]) -> bool {
    let Ok(url) = Url::parse(app_redirect) else {
        return false;
    };
    if is_loopback(&url) {
        return true;
    }
    allowed.iter().any(|origin| same_origin(&url, origin))
}

/// Whether `url` is an `http` loopback address (any port/path) — the user's own
/// machine, where the native app catches the redirect.
fn is_loopback(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    match url.host_str() {
        Some("localhost") => true,
        // `host_str` keeps the brackets on an IPv6 literal (`[::1]`); strip them
        // before parsing. A DNS name that is not exactly `localhost` never counts,
        // so `127.0.0.1.evil.com` / `localhost.evil.com` are correctly rejected.
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}

/// Whether two URLs share an origin: identical scheme, host, and effective port
/// (the scheme's default port when none is given).
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

impl<E: EphemeralStore> StatusBackend for OidcSso<E> {
    /// Probe the userinfo endpoint without a token. Any HTTP reply (typically a
    /// `401`) means the provider is reachable; a transport error means it is not.
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        match self.http.get(&self.userinfo_url).send().await {
            Ok(_) => PortStatus::Healthy,
            Err(e) => {
                tracing::warn!(error = %e, "OIDC userinfo unreachable");
                PortStatus::Unhealthy {
                    reason: format!("OIDC userinfo unreachable: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::ephemeral::InMemoryTtlEphemeralStore;

    fn provider() -> OidcSso<InMemoryTtlEphemeralStore> {
        OidcSso::new(
            "https://idp.example/userinfo".to_owned(),
            OidcFlow {
                authorize_url: "https://idp.example/authorize".to_owned(),
                token_url: "https://idp.example/token".to_owned(),
                client_id: "relatum".to_owned(),
                client_secret: "secret".to_owned(),
                scopes: "openid profile groups".to_owned(),
                public_url: "https://relatum.example".to_owned(),
                allowed_redirects: vec!["https://app.relatum.example".to_owned()],
            },
            InMemoryTtlEphemeralStore::new(),
        )
        .unwrap()
    }

    fn claims(value: Value) -> HashMap<String, Value> {
        value
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[test]
    fn maps_the_sub_claim_to_the_identity() {
        // Role/department come from the LDAP sync now; only `sub` is read here.
        let identity = provider()
            .map_claims(&claims(json!({
                "sub": "u-123",
                "preferred_username": "alice",
                "groups": ["team-blue", "trainees"],
            })))
            .unwrap();

        assert_eq!(identity.id.as_str(), "u-123");
    }

    #[test]
    fn rejects_a_token_without_a_sub() {
        let err = provider()
            .map_claims(&claims(json!({ "groups": ["team-blue"] })))
            .unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn begin_stores_pending_and_builds_an_authorize_url() {
        let provider = provider();
        let url = provider
            .begin("http://127.0.0.1:1234/?state=abc")
            .await
            .unwrap();

        assert!(url.starts_with("https://idp.example/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=relatum"));

        // The pending login is stashed in the ephemeral store under the `state` handed
        // to the IdP, carrying the app redirect for `complete` to return the browser to.
        let parsed = Url::parse(&url).unwrap();
        let state = parsed
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .expect("authorize URL carries a state");
        let stored = provider
            .ephemeral
            .take(&pending_key(&state))
            .await
            .unwrap()
            .expect("pending login stored");
        assert!(stored.contains("http://127.0.0.1:1234/?state=abc"));
    }

    #[tokio::test]
    async fn complete_rejects_an_unknown_state() {
        // No HTTP is attempted: an unknown `state` is rejected before any exchange.
        let err = provider().complete("some-code", "never-seen").await.unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn handoff_is_single_use() {
        let provider = provider();
        let code = provider.stash_handoff("access-tok".to_owned()).await.unwrap();

        // First redemption returns the stashed token...
        assert_eq!(provider.redeem_handoff(&code).await.unwrap(), "access-tok");
        // ...and consumes it, so a second redemption is rejected.
        let err = provider.redeem_handoff(&code).await.unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn redeem_rejects_an_unknown_handoff() {
        let err = provider().redeem_handoff("never-issued").await.unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[test]
    fn redirect_allowed_accepts_loopback_any_port() {
        // The native CLI's RFC 8252 callback: loopback, any port/path, http only.
        for ok in [
            "http://127.0.0.1:1234/?state=x",
            "http://127.0.0.1/cb",
            "http://localhost:9000/auth?state=x",
            "http://[::1]:5/",
        ] {
            assert!(redirect_allowed(ok, &[]), "{ok} should be allowed");
        }
    }

    #[test]
    fn redirect_allowed_accepts_a_configured_origin() {
        let allowed = [Url::parse("https://app.relatum.example").unwrap()];
        // Same origin, any path/query (the web frontend's `/auth/callback`).
        assert!(redirect_allowed(
            "https://app.relatum.example/auth/callback?state=x",
            &allowed
        ));
    }

    #[test]
    fn redirect_allowed_rejects_foreign_and_lookalike_destinations() {
        let allowed = [Url::parse("https://app.relatum.example").unwrap()];
        for bad in [
            // Outright attacker-controlled host.
            "https://evil.example/grab?state=x",
            // Look-alike host (suffix) must not match an allowed origin.
            "https://app.relatum.example.evil.com/cb",
            // Scheme mismatch (http vs https).
            "http://app.relatum.example/cb",
            // Port mismatch.
            "https://app.relatum.example:8443/cb",
            // `userinfo@` trick: the real host is evil.com.
            "https://app.relatum.example@evil.com/cb",
            // Non-loopback dressed up to look local.
            "http://127.0.0.1.evil.com/cb",
            "http://localhost.evil.com/cb",
            // Unparseable.
            "not a url",
        ] {
            assert!(!redirect_allowed(bad, &allowed), "{bad} should be rejected");
        }
    }

    #[tokio::test]
    async fn begin_rejects_a_disallowed_redirect_without_storing_state() {
        let provider = provider();
        let err = provider
            .begin("https://evil.example/grab?state=x")
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));

        // The redirect is validated *before* any state is stashed, so a rejected start
        // must leave the ephemeral store untouched — no `pending:<state>` to later
        // redeem. Guards against a future reordering that stores before validating.
        assert!(
            provider.ephemeral.is_empty(),
            "no pending state may be stored when the redirect is rejected"
        );
    }

    #[test]
    fn new_rejects_an_unparseable_allowed_redirect() {
        // A malformed allowlist entry must fail construction (i.e. server startup),
        // not be silently skipped — that's the point of parsing once, up front.
        let flow = OidcFlow {
            authorize_url: "https://idp.example/authorize".to_owned(),
            token_url: "https://idp.example/token".to_owned(),
            client_id: "relatum".to_owned(),
            client_secret: "secret".to_owned(),
            scopes: "openid".to_owned(),
            public_url: "https://relatum.example".to_owned(),
            allowed_redirects: vec!["not a url".to_owned()],
        };
        let result = OidcSso::new(
            "https://idp.example/userinfo".to_owned(),
            flow,
            InMemoryTtlEphemeralStore::new(),
        );
        assert!(matches!(result, Err(DomainError::Backend(_))));
    }
}
