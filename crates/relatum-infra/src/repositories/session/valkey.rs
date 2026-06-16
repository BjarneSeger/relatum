//! Valkey/Redis-backed [`SessionRepository`].
//!
//! Tokens are written as keys with a native TTL (`SET … EX`), so expiry is
//! enforced by the server and `lookup` needs no manual eviction. Compiled only
//! when the `valkey` feature is enabled.

use std::time::Duration;

use redis::AsyncCommands;
use relatum_domain::errors::DomainError;
use relatum_domain::models::auth::SessionToken;
use relatum_domain::models::ids::UserId;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::status::{PortStatus, StatusBackend};

/// Prefix applied to every key, namespacing sessions within the keyspace.
const KEY_PREFIX: &str = "session:";

/// Persists session tokens in Valkey (or any Redis-compatible server).
///
/// Holds a multiplexed [`ConnectionManager`] that is cheap to clone and
/// auto-reconnects, so the store is `Clone + Send + Sync` and fits behind shared
/// async state.
///
/// [`ConnectionManager`]: redis::aio::ConnectionManager
#[derive(Clone)]
pub struct ValkeySessionStore {
    conn: redis::aio::ConnectionManager,
    ttl: Duration,
}

impl ValkeySessionStore {
    /// Connect to the server at `url` (e.g. `redis://127.0.0.1:6379`). Stored
    /// tokens expire `ttl` after they are written.
    pub async fn connect(url: &str, ttl: Duration) -> Result<Self, DomainError> {
        let client = redis::Client::open(url)
            .map_err(|e| DomainError::Backend(format!("invalid valkey url: {e}")))?;
        let conn = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| DomainError::Backend(format!("valkey connection failed: {e}")))?;
        Ok(Self { conn, ttl })
    }

    /// The namespaced key under which `value` is stored.
    fn key(value: &str) -> String {
        format!("{KEY_PREFIX}{value}")
    }
}

impl SessionRepository for ValkeySessionStore {
    // `token`/`value` carry the bearer secret, so they are skipped; only the subject
    // (a user id) is surfaced, on store.
    #[tracing::instrument(skip(self, token), fields(user_id = %token.subject.as_str()), level = "debug")]
    async fn store(&self, token: &SessionToken) -> Result<(), DomainError> {
        let mut conn = self.conn.clone();
        // The token value is the key; its subject is the stored payload, so a
        // presented token can be resolved back to the user it authenticates.
        let _: () = conn
            .set_ex(
                Self::key(&token.value),
                token.subject.as_str(),
                self.ttl.as_secs(),
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "valkey session store failed");
                DomainError::Backend(format!("valkey store failed: {e}"))
            })?;
        Ok(())
    }

    #[tracing::instrument(skip(self, value), level = "debug")]
    async fn lookup(&self, value: &str) -> Result<Option<SessionToken>, DomainError> {
        let mut conn = self.conn.clone();
        let found: Option<String> = conn.get(Self::key(value)).await.map_err(|e| {
            tracing::error!(error = %e, "valkey session lookup failed");
            DomainError::Backend(format!("valkey lookup failed: {e}"))
        })?;
        // A missing key (expired or never stored) comes back as `None`; the stored
        // payload is the token's subject.
        if found.is_none() {
            tracing::debug!("session not found");
        }
        Ok(found.map(|subject| SessionToken {
            value: value.to_owned(),
            subject: UserId::new(subject),
        }))
    }

    #[tracing::instrument(skip(self, value), level = "debug")]
    async fn revoke(&self, value: &str) -> Result<(), DomainError> {
        let mut conn = self.conn.clone();
        let _: () = conn.del(Self::key(value)).await.map_err(|e| {
            tracing::error!(error = %e, "valkey session revoke failed");
            DomainError::Backend(format!("valkey revoke failed: {e}"))
        })?;
        Ok(())
    }
}

impl StatusBackend for ValkeySessionStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        let mut conn = self.conn.clone();
        let pong: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        match pong {
            Ok(reply) if reply == "PONG" => PortStatus::Healthy,
            Ok(reply) => {
                tracing::warn!(%reply, "unexpected valkey PING reply (session store)");
                PortStatus::Unhealthy {
                    reason: format!("unexpected PING reply: {reply}"),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "valkey session-store ping failed");
                PortStatus::Unhealthy {
                    reason: format!("valkey ping failed: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end against a real server. Ignored by default (it needs a running
    /// Valkey/Redis); run with:
    /// `VALKEY_TEST_URL=redis://127.0.0.1:6379 cargo test -p relatum-infra --features valkey -- --ignored`
    #[tokio::test]
    #[ignore = "requires a running Valkey/Redis server"]
    async fn store_lookup_revoke_roundtrip() {
        let url = std::env::var("VALKEY_TEST_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        let store = ValkeySessionStore::connect(&url, Duration::from_secs(60))
            .await
            .expect("connect to valkey");

        assert!(matches!(store.get_status().await, PortStatus::Healthy));

        // A unique key keeps concurrent test runs from colliding.
        let value = format!("test-{}", uuid::Uuid::new_v4().simple());
        let tok = SessionToken {
            value: value.clone(),
            subject: UserId::new("tester"),
        };

        store.store(&tok).await.unwrap();
        assert_eq!(
            store.lookup(&value).await.unwrap().map(|t| t.value),
            Some(value.clone())
        );

        store.revoke(&value).await.unwrap();
        assert!(store.lookup(&value).await.unwrap().is_none());
    }
}
