//! In-memory [`EphemeralStore`] backed by a `HashMap` with per-entry TTL.
//!
//! Self-contained: it needs no external service, so it is the natural default for
//! development and single-node deployments. Each stored value carries an expiry
//! computed from the TTL passed to [`put`]; an entry past its expiry is treated as
//! absent and dropped on access. A [`take`] removes its entry as it returns it, so a
//! value is read at most once. An optional background sweeper
//! ([`InMemoryTtlEphemeralStore::spawn_sweeper`]) reclaims memory for values that are
//! stored and then never taken (e.g. a login that is started but never completed).
//!
//! [`put`]: EphemeralStore::put
//! [`take`]: EphemeralStore::take

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use relatum_domain::errors::DomainError;
use relatum_domain::ports::ephemeral::EphemeralStore;
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use tokio::time::Instant;

/// A stored value together with the instant it expires.
struct Entry {
    value: String,
    expires_at: Instant,
}

/// In-memory [`EphemeralStore`] keyed by an opaque string, with per-entry TTL.
///
/// Cheap to clone — clones share one `Arc<Mutex<…>>` — so the store can be handed to
/// several handlers and to its own background sweeper. It is `Send + Sync`: the mutex
/// guard is never held across an `.await`, so the in-trait async futures stay `Send`.
#[derive(Clone, Default)]
pub struct InMemoryTtlEphemeralStore {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

impl InMemoryTtlEphemeralStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a background task that evicts expired entries every `interval`.
    ///
    /// Lazy eviction on [`take`] already keeps results correct; the sweeper only
    /// reclaims memory for values that are stored and then never taken. Requires a
    /// running Tokio runtime. Drop the returned handle to let it run detached, or
    /// `abort` it to stop sweeping.
    ///
    /// [`take`]: EphemeralStore::take
    pub fn spawn_sweeper(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        let entries = Arc::clone(&self.entries);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let now = Instant::now();
                entries
                    .lock()
                    .unwrap()
                    .retain(|_, entry| entry.expires_at > now);
            }
        })
    }
}

#[cfg(test)]
impl InMemoryTtlEphemeralStore {
    /// Whether the store holds no entries. Test-only inspection so callers in other
    /// modules (e.g. the SSO flow tests) can assert nothing was stashed.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }
}

impl EphemeralStore for InMemoryTtlEphemeralStore {
    async fn put(&self, key: &str, value: &str, ttl: Duration) -> Result<(), DomainError> {
        let expires_at = Instant::now() + ttl;
        self.entries.lock().unwrap().insert(
            key.to_owned(),
            Entry {
                value: value.to_owned(),
                expires_at,
            },
        );
        Ok(())
    }

    async fn take(&self, key: &str) -> Result<Option<String>, DomainError> {
        // Remove unconditionally: a present entry is consumed (single-use), and an
        // expired one is dropped either way.
        match self.entries.lock().unwrap().remove(key) {
            Some(entry) if entry.expires_at > Instant::now() => Ok(Some(entry.value)),
            _ => Ok(None),
        }
    }
}

impl StatusBackend for InMemoryTtlEphemeralStore {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn stores_and_takes_a_value() {
        let store = InMemoryTtlEphemeralStore::new();
        store.put("k", "v", Duration::from_secs(60)).await.unwrap();

        assert_eq!(store.take("k").await.unwrap().as_deref(), Some("v"));
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_key_is_none() {
        let store = InMemoryTtlEphemeralStore::new();
        assert!(store.take("nope").await.unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn take_consumes_the_value() {
        let store = InMemoryTtlEphemeralStore::new();
        store.put("k", "v", Duration::from_secs(60)).await.unwrap();

        // First take returns it…
        assert_eq!(store.take("k").await.unwrap().as_deref(), Some("v"));
        // …and a second take finds nothing: single-use.
        assert!(store.take("k").await.unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn value_expires_after_its_ttl() {
        let ttl = Duration::from_secs(60);
        let store = InMemoryTtlEphemeralStore::new();
        store.put("k", "v", ttl).await.unwrap();

        // Just before expiry it is still readable…
        tokio::time::advance(ttl - Duration::from_secs(1)).await;
        // (peek without consuming by re-storing afterwards is overkill; just assert
        // a fresh entry is still live by taking it.)
        assert_eq!(store.take("k").await.unwrap().as_deref(), Some("v"));

        // …and once the lifetime elapses a stored value is gone.
        store.put("k2", "v2", ttl).await.unwrap();
        tokio::time::advance(ttl + Duration::from_secs(1)).await;
        assert!(store.take("k2").await.unwrap().is_none());
        assert!(!store.entries.lock().unwrap().contains_key("k2"));
    }
}
