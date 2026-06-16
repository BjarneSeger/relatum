//! [`SSOProvider`] adapters.
//!
//! Two interchangeable implementations, mirroring the storage ports' shape: the
//! self-contained [`DisabledSso`] (no upstream IdP — every login is rejected,
//! since authentication is SSO-only), and the feature-gated [`OidcSso`] behind
//! `oidc`, which validates tokens against an OpenID Connect provider.
//!
//! [`SSOProvider`]: relatum_domain::ports::sso_connector::SSOProvider

pub mod disabled;
pub use disabled::DisabledSso;

#[cfg(feature = "oidc")]
pub mod oidc;
#[cfg(feature = "oidc")]
pub use oidc::{OidcFlow, OidcSso};
