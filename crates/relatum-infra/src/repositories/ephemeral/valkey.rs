//! Valkey/Redis-backed [`EphemeralStore`].
//!
//! Values are written as keys with a native TTL (`SET … EX`), so expiry is enforced
//! by the server and `take` needs no manual eviction. `take` uses `GETDEL`, which
//! reads and deletes in one atomic step, so a value is handed to at most one caller
//! even when several replicas race to redeem it. Because the state lives in the
//! shared server rather than one process, an SSO login started on one replica can be
//! completed on another. Compiled only when the `valkey` feature is enabled.

use std::time::Duration;

use redis::AsyncCommands;
use relatum_domain::errors::DomainError;
use relatum_domain::ports::ephemeral::EphemeralStore;
use relatum_domain::ports::status::{PortStatus, StatusBackend};

/// Prefix applied to every key, namespacing this store within the keyspace.
const KEY_PREFIX: &str = "sso:";

/// Persists short-lived values in Valkey (or any Redis-compatible server).
///
/// Holds a multiplexed [`ConnectionManager`] that is cheap to clone and
/// auto-reconnects, so the store is `Clone + Send + Sync` and fits behind shared
/// async state. Unlike the session store, the lifetime is supplied per [`put`] call
/// (the pending and handoff windows differ), so no TTL is held here.
///
/// [`ConnectionManager`]: redis::aio::ConnectionManager
/// [`put`]: EphemeralStore::put
#[derive(Clone)]
pub struct ValkeyEphemeralStore {
    conn: redis::aio::ConnectionManager,
}

impl ValkeyEphemeralStore {
    /// Connect to the server at `url` (e.g. `redis://127.0.0.1:6379`).
    pub async fn connect(url: &str) -> Result<Self, DomainError> {
        let client = redis::Client::open(url)
            .map_err(|e| DomainError::Backend(format!("invalid valkey url: {e}")))?;
        let conn = redis::aio::ConnectionManager::new(client)
            .await
            .map_err(|e| DomainError::Backend(format!("valkey connection failed: {e}")))?;
        Ok(Self { conn })
    }

    /// The namespaced key under which `key` is stored.
    fn key(key: &str) -> String {
        format!("{KEY_PREFIX}{key}")
    }
}

impl EphemeralStore for ValkeyEphemeralStore {
    // `key` is derived from the SSO `state`/handoff code and `value` holds the PKCE
    // verifier / access token — both secret, so both are skipped (only the TTL is logged).
    #[tracing::instrument(skip(self, key, value, ttl), fields(ttl_secs = ttl.as_secs()), level = "debug")]
    async fn put(&self, key: &str, value: &str, ttl: Duration) -> Result<(), DomainError> {
        let mut conn = self.conn.clone();
        let _: () = conn
            .set_ex(Self::key(key), value, ttl.as_secs())
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "valkey ephemeral put failed");
                DomainError::Backend(format!("valkey put failed: {e}"))
            })?;
        Ok(())
    }

    #[tracing::instrument(skip(self, key), level = "debug")]
    async fn take(&self, key: &str) -> Result<Option<String>, DomainError> {
        let mut conn = self.conn.clone();
        // GETDEL reads and removes atomically, so a value is consumed exactly once.
        // A missing key (expired or already taken) comes back as `None`.
        let found: Option<String> = conn.get_del(Self::key(key)).await.map_err(|e| {
            tracing::error!(error = %e, "valkey ephemeral take failed");
            DomainError::Backend(format!("valkey take failed: {e}"))
        })?;
        if found.is_none() {
            tracing::debug!("ephemeral key absent");
        }
        Ok(found)
    }
}

impl StatusBackend for ValkeyEphemeralStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        let mut conn = self.conn.clone();
        let pong: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        match pong {
            Ok(reply) if reply == "PONG" => PortStatus::Healthy,
            Ok(reply) => {
                tracing::warn!(%reply, "unexpected valkey PING reply (ephemeral store)");
                PortStatus::Unhealthy {
                    reason: format!("unexpected PING reply: {reply}"),
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "valkey ephemeral-store ping failed");
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
    async fn put_take_is_single_use() {
        let url = std::env::var("VALKEY_TEST_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        let store = ValkeyEphemeralStore::connect(&url)
            .await
            .expect("connect to valkey");

        assert!(matches!(store.get_status().await, PortStatus::Healthy));

        // A unique key keeps concurrent test runs from colliding.
        let key = format!("test-{}", uuid::Uuid::new_v4().simple());
        store
            .put(&key, "payload", Duration::from_secs(60))
            .await
            .unwrap();

        // First take returns the value and consumes it…
        assert_eq!(store.take(&key).await.unwrap().as_deref(), Some("payload"));
        // …so a second take finds nothing.
        assert!(store.take(&key).await.unwrap().is_none());
    }
}
