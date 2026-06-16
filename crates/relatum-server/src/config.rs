//! Startup configuration: layered defaults → file → environment.
//!
//! Built with [`confique`]: every option has a default and a `RELATUM_*`
//! environment override, and the documented TOML template is generated from this
//! type (see [`template`]). The config file is **optional** — the server can boot
//! from environment variables and defaults alone.
//!
//! Each storage section names its `backend` and carries the fields that backend
//! needs. [`ServerConfig::load`] validates the combination (the `postgres` backend
//! requires a `url`, `redis` likewise) and lowers it to the runtime [`DataStore`] /
//! [`SessionStore`] the binary dispatches on.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use confique::Config;
use serde::Deserialize;

/// Server configuration as merged from the default, file and environment layers.
#[derive(Debug, Config)]
pub struct ServerConfig {
    /// Address the HTTP server binds to.
    #[config(env = "RELATUM_LISTEN", default = "0.0.0.0:8080")]
    pub listen: SocketAddr,

    /// Backing store for users and reports.
    #[config(nested)]
    pub data: DataConfig,

    /// Backing store for session tokens.
    #[config(nested)]
    pub sessions: SessionsConfig,

    /// Single-sign-on provider.
    #[config(nested)]
    pub sso: SsoConfig,

    /// External directory (LDAP) the periodic user sync provisions from.
    #[config(nested)]
    pub directory: DirectoryConfig,

    /// The fixed set of departments users may be assigned to. Hard-coded at
    /// startup; an assignment to any other department is rejected. Set from the
    /// environment as a comma-separated list, e.g. `RELATUM_DEPARTMENTS=blue,red`.
    #[config(
        env = "RELATUM_DEPARTMENTS",
        parse_env = confique::env::parse::list_by_comma,
        default = []
    )]
    pub departments: Vec<String>,
}

/// The data store (users + reports). One backend serves both, since the report
/// workflow reads users too.
#[derive(Debug, Config)]
pub struct DataConfig {
    /// Which backend stores users and reports: `memory` or `postgres`.
    #[config(env = "RELATUM_DATA_BACKEND")]
    pub backend: Option<DataBackend>,

    /// Connection URL, required when `backend = "postgres"`.
    /// e.g. `postgres://user:pass@host:5432/dbname`.
    #[config(env = "RELATUM_DATA_URL")]
    pub url: Option<String>,
}

/// The session store.
#[derive(Debug, Config)]
pub struct SessionsConfig {
    /// Which backend stores session tokens: `memory` or `redis`.
    #[config(env = "RELATUM_SESSIONS_BACKEND")]
    pub backend: Option<SessionBackend>,

    /// Connection URL, required when `backend = "redis"`.
    /// e.g. `redis://127.0.0.1:6379`.
    #[config(env = "RELATUM_SESSIONS_URL")]
    pub url: Option<String>,

    /// Session lifetime in seconds.
    #[config(env = "RELATUM_SESSIONS_TTL_SECS", default = 86400)]
    pub ttl_secs: u64,
}

/// The single-sign-on provider.
#[derive(Debug, Config)]
pub struct SsoConfig {
    /// Which SSO provider validates tokens: `disabled` or `oidc`.
    #[config(env = "RELATUM_SSO_BACKEND")]
    pub backend: Option<SsoBackend>,

    /// OIDC userinfo endpoint, required when `backend = "oidc"`. The submitted
    /// bearer token is validated against it. e.g.
    /// `https://idp.example/realms/r/protocol/openid-connect/userinfo`.
    #[config(env = "RELATUM_SSO_USERINFO_URL")]
    pub userinfo_url: Option<String>,

    /// IdP authorization endpoint, required when `backend = "oidc"`. The browser is
    /// sent here to log in. e.g.
    /// `https://idp.example/realms/r/protocol/openid-connect/auth`.
    #[config(env = "RELATUM_SSO_AUTHORIZE_URL")]
    pub authorize_url: Option<String>,

    /// IdP token endpoint, required when `backend = "oidc"`. The authorization code
    /// is exchanged here for an access token. e.g.
    /// `https://idp.example/realms/r/protocol/openid-connect/token`.
    #[config(env = "RELATUM_SSO_TOKEN_URL")]
    pub token_url: Option<String>,

    /// OAuth client id registered with the IdP, required when `backend = "oidc"`.
    #[config(env = "RELATUM_SSO_CLIENT_ID")]
    pub client_id: Option<String>,

    /// OAuth client secret for the confidential client, required when
    /// `backend = "oidc"`.
    #[config(env = "RELATUM_SSO_CLIENT_SECRET")]
    pub client_secret: Option<String>,

    /// Space-separated OAuth scopes to request during login.
    #[config(env = "RELATUM_SSO_SCOPES", default = "openid profile groups")]
    pub scopes: String,

    /// The server's externally reachable base URL, required when `backend = "oidc"`.
    /// Used to build the redirect URI `{public_url}/api/v1/auth/sso/callback` that
    /// must be registered with the IdP. e.g. `https://relatum.example`.
    #[config(env = "RELATUM_SSO_PUBLIC_URL")]
    pub public_url: Option<String>,

    /// Origins (`scheme://host[:port]`) the SSO browser flow may return to once
    /// login completes — set this to the web frontend's origin, e.g.
    /// `https://relatum.example`. The login result is only ever delivered to one of
    /// these or to a loopback address (the native CLI is always allowed); any other
    /// `redirect_uri` is rejected, closing an open-redirect → account-takeover hole.
    /// Comma-separated. Only the origin (scheme, host, port) of each entry matters;
    /// any path is ignored (`https://app.example.com` ≡ `https://app.example.com/app`).
    ///
    /// With none set, only loopback redirects are accepted, so the web frontend's
    /// origin **must** be listed for browser SSO to work. **Upgrade note:** this
    /// defaults to empty — a deployment that previously relied on browser SSO must set
    /// it after upgrading, or browser logins will be rejected (the native CLI keeps
    /// working). The server logs a startup warning when SSO is on and this is empty.
    #[config(
        env = "RELATUM_SSO_ALLOWED_REDIRECTS",
        parse_env = confique::env::parse::list_by_comma,
        default = []
    )]
    pub allowed_redirects: Vec<String>,
}

/// The external directory users are provisioned from.
///
/// A periodic sync reconciles the directory into the user store: entries appear as
/// inert users (no department until an admin assigns one) and vanish when removed
/// from the directory. With `backend = "disabled"` no sync runs and users must be
/// present by other means.
#[derive(Debug, Config)]
pub struct DirectoryConfig {
    /// Which directory the user sync reads: `disabled` or `ldap`.
    #[config(env = "RELATUM_DIRECTORY_BACKEND")]
    pub backend: Option<DirectoryBackend>,

    /// LDAP server URL, required when `backend = "ldap"`. e.g.
    /// `ldaps://ldap.example:636`.
    #[config(env = "RELATUM_DIRECTORY_URL")]
    pub url: Option<String>,

    /// Bind DN for the service account used to search the directory. Empty means an
    /// anonymous bind.
    #[config(env = "RELATUM_DIRECTORY_BIND_DN", default = "")]
    pub bind_dn: String,

    /// Password for the bind DN. Ignored for an anonymous bind.
    #[config(env = "RELATUM_DIRECTORY_BIND_PASSWORD", default = "")]
    pub bind_password: String,

    /// Search base under which user entries live, required when `backend = "ldap"`.
    /// e.g. `ou=people,dc=example,dc=org`.
    #[config(env = "RELATUM_DIRECTORY_USER_BASE")]
    pub user_base: Option<String>,

    /// Search filter selecting user entries.
    #[config(
        env = "RELATUM_DIRECTORY_USER_FILTER",
        default = "(objectClass=person)"
    )]
    pub user_filter: String,

    /// Entry attribute whose value becomes the user id — must match the subject an
    /// SSO token attests (e.g. `uid` or `entryUUID`).
    #[config(env = "RELATUM_DIRECTORY_ID_ATTR", default = "uid")]
    pub id_attr: String,

    /// Entry attribute whose value becomes the login username.
    #[config(env = "RELATUM_DIRECTORY_USERNAME_ATTR", default = "uid")]
    pub username_attr: String,

    /// Membership attribute inspected for group markers (e.g. `memberOf`).
    #[config(env = "RELATUM_DIRECTORY_GROUP_ATTR", default = "memberOf")]
    pub group_attr: String,

    /// Group whose members map to the instructor role, required when
    /// `backend = "ldap"`.
    #[config(env = "RELATUM_DIRECTORY_INSTRUCTOR_GROUP")]
    pub instructor_group: Option<String>,

    /// Group whose members map to the trainee role, required when `backend = "ldap"`.
    #[config(env = "RELATUM_DIRECTORY_TRAINEE_GROUP")]
    pub trainee_group: Option<String>,

    /// How often (seconds) to re-run the directory sync.
    #[config(env = "RELATUM_DIRECTORY_SYNC_INTERVAL_SECS", default = 3600)]
    pub sync_interval_secs: u64,
}

/// Selectable backend for the data store. Deserialized from the lowercase name in
/// both the file and the environment variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataBackend {
    /// In-process `HashMap` stores; no external service.
    #[default]
    Memory,
    /// PostgreSQL-backed user and report stores.
    Postgres,
}

/// Selectable backend for the session store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionBackend {
    /// In-process store with per-entry TTL; no external service.
    #[default]
    Memory,
    /// Valkey/Redis-backed store with native key expiry.
    Redis,
}

/// Selectable single-sign-on provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SsoBackend {
    /// No external IdP; all logins are rejected (authentication is SSO-only).
    #[default]
    Disabled,
    /// OpenID Connect provider validated against its userinfo endpoint.
    Oidc,
    /// In-process mock provider attesting a fixed set of dev tokens. Dev builds only.
    #[cfg(feature = "dev")]
    Mock,
}

/// Selectable external directory backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryBackend {
    /// No directory; the periodic sync does not run.
    #[default]
    Disabled,
    /// LDAP-backed directory source.
    Ldap,
    /// Seed a fixed set of dev users directly into the store. Dev builds only.
    #[cfg(feature = "dev")]
    Mock,
}

/// The validated, runtime-ready configuration the binary dispatches on.
#[derive(Debug)]
pub struct Resolved {
    /// Address the HTTP server binds to.
    pub listen: SocketAddr,
    /// Chosen data store, with the fields its backend needs.
    pub data: DataStore,
    /// Chosen session store, with the fields its backend needs.
    pub sessions: SessionStore,
    /// Chosen single-sign-on provider, with the fields its backend needs.
    pub sso: SsoStore,
    /// Chosen external directory, with the fields its backend needs.
    pub directory: DirectoryStore,
    /// The fixed set of departments users may be assigned to.
    pub departments: Vec<String>,
}

/// Where users and reports are persisted, carrying only the chosen backend's fields.
#[derive(Debug)]
pub enum DataStore {
    /// In-process stores.
    Memory,
    /// PostgreSQL-backed stores.
    Postgres { url: String },
}

/// Where session tokens are persisted, carrying only the chosen backend's fields.
#[derive(Debug)]
pub enum SessionStore {
    /// In-process TTL store.
    Memory { ttl_secs: u64 },
    /// Valkey/Redis-backed store.
    Redis { url: String, ttl_secs: u64 },
}

/// Which SSO provider validates tokens, carrying only the chosen backend's fields.
#[derive(Debug)]
pub enum SsoStore {
    /// No external IdP.
    Disabled,
    /// OIDC userinfo-backed provider with the authorization-code-flow
    /// endpoints/credentials for driving browser login.
    Oidc {
        userinfo_url: String,
        authorize_url: String,
        token_url: String,
        client_id: String,
        client_secret: String,
        scopes: String,
        public_url: String,
        /// Allowlist of origins the browser flow may return to (plus loopback).
        allowed_redirects: Vec<String>,
    },
    /// In-process mock provider seeded with the dev tokens. Dev builds only.
    #[cfg(feature = "dev")]
    Mock,
}

/// Which external directory the user sync reads, carrying only the chosen backend's
/// fields.
#[derive(Debug)]
pub enum DirectoryStore {
    /// No directory; the periodic sync does not run.
    Disabled,
    /// LDAP-backed directory. Boxed because its payload dwarfs the `Disabled`
    /// variant; the value is built once at startup, so the indirection is free.
    Ldap(Box<LdapSettings>),
    /// Seed the fixed set of dev users directly into the store. Dev builds only.
    #[cfg(feature = "dev")]
    Mock,
}

/// The validated LDAP directory settings: the connection/search configuration plus
/// the reconciliation interval.
#[derive(Debug)]
pub struct LdapSettings {
    pub url: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub user_base: String,
    pub user_filter: String,
    pub id_attr: String,
    pub username_attr: String,
    pub group_attr: String,
    pub instructor_group: String,
    pub trainee_group: String,
    pub interval_secs: u64,
}

impl SessionStore {
    /// The configured session lifetime, regardless of backend.
    pub fn ttl(&self) -> Duration {
        let secs = match self {
            SessionStore::Memory { ttl_secs } | SessionStore::Redis { ttl_secs, .. } => *ttl_secs,
        };
        Duration::from_secs(secs)
    }
}

impl ServerConfig {
    /// Load and validate the configuration.
    ///
    /// Layers, lowest priority first: built-in defaults, then the file at `file`
    /// (or `config.toml` in the working directory), then `RELATUM_*` environment
    /// variables — so the environment wins and a missing file is ignored.
    pub fn load(file: Option<&Path>) -> anyhow::Result<Resolved> {
        let builder = ServerConfig::builder().env();
        let builder = match file {
            Some(path) => builder.file(path),
            None => builder.file("config.toml"),
        };
        builder.load().context("loading configuration")?.resolve()
    }

    /// Validate the backend/field combinations and lower to [`Resolved`].
    fn resolve(self) -> anyhow::Result<Resolved> {
        let data = match self.data.backend.unwrap_or_default() {
            DataBackend::Memory => DataStore::Memory,
            DataBackend::Postgres => DataStore::Postgres {
                url: self.data.url.context(
                    "`data.url` (or RELATUM_DATA_URL) is required when data.backend = \"postgres\"",
                )?,
            },
        };
        let sessions = match self.sessions.backend.unwrap_or_default() {
            SessionBackend::Memory => SessionStore::Memory {
                ttl_secs: self.sessions.ttl_secs,
            },
            SessionBackend::Redis => SessionStore::Redis {
                url: self.sessions.url.context(
                    "`sessions.url` (or RELATUM_SESSIONS_URL) is required when sessions.backend = \"redis\"",
                )?,
                ttl_secs: self.sessions.ttl_secs,
            },
        };
        let sso = match self.sso.backend.unwrap_or_default() {
            SsoBackend::Disabled => SsoStore::Disabled,
            SsoBackend::Oidc => SsoStore::Oidc {
                userinfo_url: self.sso.userinfo_url.context(
                    "`sso.userinfo_url` (or RELATUM_SSO_USERINFO_URL) is required when sso.backend = \"oidc\"",
                )?,
                authorize_url: self.sso.authorize_url.context(
                    "`sso.authorize_url` (or RELATUM_SSO_AUTHORIZE_URL) is required when sso.backend = \"oidc\"",
                )?,
                token_url: self.sso.token_url.context(
                    "`sso.token_url` (or RELATUM_SSO_TOKEN_URL) is required when sso.backend = \"oidc\"",
                )?,
                client_id: self.sso.client_id.context(
                    "`sso.client_id` (or RELATUM_SSO_CLIENT_ID) is required when sso.backend = \"oidc\"",
                )?,
                client_secret: self.sso.client_secret.context(
                    "`sso.client_secret` (or RELATUM_SSO_CLIENT_SECRET) is required when sso.backend = \"oidc\"",
                )?,
                scopes: self.sso.scopes,
                public_url: self.sso.public_url.context(
                    "`sso.public_url` (or RELATUM_SSO_PUBLIC_URL) is required when sso.backend = \"oidc\"",
                )?,
                allowed_redirects: clean_list(self.sso.allowed_redirects),
            },
            #[cfg(feature = "dev")]
            SsoBackend::Mock => SsoStore::Mock,
        };
        let directory = match self.directory.backend.unwrap_or_default() {
            DirectoryBackend::Disabled => DirectoryStore::Disabled,
            DirectoryBackend::Ldap => DirectoryStore::Ldap(Box::new(LdapSettings {
                url: self.directory.url.context(
                    "`directory.url` (or RELATUM_DIRECTORY_URL) is required when directory.backend = \"ldap\"",
                )?,
                bind_dn: self.directory.bind_dn,
                bind_password: self.directory.bind_password,
                user_base: self.directory.user_base.context(
                    "`directory.user_base` (or RELATUM_DIRECTORY_USER_BASE) is required when directory.backend = \"ldap\"",
                )?,
                user_filter: self.directory.user_filter,
                id_attr: self.directory.id_attr,
                username_attr: self.directory.username_attr,
                group_attr: self.directory.group_attr,
                instructor_group: self.directory.instructor_group.context(
                    "`directory.instructor_group` (or RELATUM_DIRECTORY_INSTRUCTOR_GROUP) is required when directory.backend = \"ldap\"",
                )?,
                trainee_group: self.directory.trainee_group.context(
                    "`directory.trainee_group` (or RELATUM_DIRECTORY_TRAINEE_GROUP) is required when directory.backend = \"ldap\"",
                )?,
                interval_secs: self.directory.sync_interval_secs,
            })),
            #[cfg(feature = "dev")]
            DirectoryBackend::Mock => DirectoryStore::Mock,
        };
        let departments = clean_list(self.departments);

        Ok(Resolved {
            listen: self.listen,
            data,
            sessions,
            sso,
            directory,
            departments,
        })
    }
}

/// Normalize a comma-separated config list: trim each entry and drop blanks.
///
/// `confique`'s `list_by_comma` keeps empty fields, so a stray comma or a
/// quoted-empty value would otherwise inject an empty string into the list. That must
/// never happen for the security-relevant `departments` and `allowed_redirects`
/// allowlists, where an empty entry could silently widen the allowed set.
fn clean_list(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// The documented TOML config template, listing every option with its default and
/// `RELATUM_*` environment variable. Backs the `generate-config` subcommand.
pub fn template() -> String {
    confique::toml::template::<ServerConfig>(confique::toml::FormatOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_list_trims_and_drops_blank_entries() {
        let cleaned = clean_list(vec![
            "  blue  ".to_owned(),
            String::new(),
            "red".to_owned(),
            "   ".to_owned(),
        ]);
        assert_eq!(cleaned, vec!["blue".to_owned(), "red".to_owned()]);
    }

    #[test]
    fn clean_list_of_only_blanks_is_empty() {
        // A lone comma or quoted-empty env value must not inject an empty-string entry
        // into a security-relevant allowlist (departments / allowed_redirects).
        assert!(clean_list(vec![String::new(), "  ".to_owned()]).is_empty());
        assert!(clean_list(vec![]).is_empty());
    }
}
