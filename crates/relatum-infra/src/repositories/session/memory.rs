//! In-memory [`SessionRepository`] backed by a `HashMap` with per-entry TTL.
//!
//! Self-contained: it needs no external service, so it is the natural default
//! for development and single-node deployments. Each stored token carries an
//! expiry computed from the store's configured lifetime; an entry past its
//! expiry is treated as absent (`lookup` returns `None`) and dropped lazily on
//! access. An optional background sweeper ([`InMemoryTtlSessionStore::spawn_sweeper`])
//! reclaims memory for tokens that are stored and then never looked up again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use relatum_domain::errors::DomainError;
use relatum_domain::models::auth::SessionToken;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use tokio::time::Instant;

/// Session lifetime used by [`InMemoryTtlSessionStore::default`].
const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// A stored token together with the instant it expires.
struct Entry {
    token: SessionToken,
    expires_at: Instant,
}

/// In-memory [`SessionRepository`] keyed by token value, with per-entry TTL.
///
/// Cheap to clone — clones share one `Arc<Mutex<…>>` — so the store can be handed
/// to several handlers and to its own background sweeper. It is `Send + Sync`:
/// the mutex guard is never held across an `.await`, so the in-trait async
/// futures stay `Send`.
#[derive(Clone)]
pub struct InMemoryTtlSessionStore {
    ttl: Duration,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

impl InMemoryTtlSessionStore {
    /// Create a store whose tokens expire `ttl` after they are stored.
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn a background task that evicts expired entries every `interval`.
    ///
    /// Lazy eviction on [`lookup`] already keeps results correct; the sweeper
    /// only reclaims memory for tokens that are stored and then never looked up
    /// again. Requires a running Tokio runtime. Drop the returned handle to let
    /// it run detached, or `abort` it to stop sweeping.
    ///
    /// [`lookup`]: SessionRepository::lookup
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

impl Default for InMemoryTtlSessionStore {
    fn default() -> Self {
        Self::new(DEFAULT_TTL)
    }
}

impl SessionRepository for InMemoryTtlSessionStore {
    async fn store(&self, token: &SessionToken) -> Result<(), DomainError> {
        let expires_at = Instant::now() + self.ttl;
        self.entries.lock().unwrap().insert(
            token.value.clone(),
            Entry {
                token: token.clone(),
                expires_at,
            },
        );
        Ok(())
    }

    async fn lookup(&self, value: &str) -> Result<Option<SessionToken>, DomainError> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get(value) {
            if entry.expires_at > Instant::now() {
                return Ok(Some(entry.token.clone()));
            }
            // Expired: fall through and drop it lazily below.
        } else {
            return Ok(None);
        }
        entries.remove(value);
        Ok(None)
    }

    async fn revoke(&self, value: &str) -> Result<(), DomainError> {
        self.entries.lock().unwrap().remove(value);
        Ok(())
    }
}

impl StatusBackend for InMemoryTtlSessionStore {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relatum_domain::models::ids::UserId;

    fn token(value: &str) -> SessionToken {
        SessionToken {
            value: value.to_owned(),
            subject: UserId::new("tester"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stores_and_looks_up_a_token() {
        let store = InMemoryTtlSessionStore::new(Duration::from_secs(60));
        store.store(&token("abc")).await.unwrap();

        let found = store.lookup("abc").await.unwrap().expect("token present");
        assert_eq!(found.value, "abc");
        // The subject round-trips so the token can be resolved back to its user.
        assert_eq!(found.subject, UserId::new("tester"));
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_token_is_none() {
        let store = InMemoryTtlSessionStore::new(Duration::from_secs(60));
        assert!(store.lookup("nope").await.unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn revoke_removes_the_token() {
        let store = InMemoryTtlSessionStore::new(Duration::from_secs(60));
        store.store(&token("abc")).await.unwrap();

        store.revoke("abc").await.unwrap();
        assert!(store.lookup("abc").await.unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn token_expires_after_its_ttl() {
        let ttl = Duration::from_secs(60);
        let store = InMemoryTtlSessionStore::new(ttl);
        store.store(&token("abc")).await.unwrap();

        // Just before expiry it is still valid…
        tokio::time::advance(ttl - Duration::from_secs(1)).await;
        assert!(store.lookup("abc").await.unwrap().is_some());

        // …and once the lifetime elapses it is gone — and evicted from the map.
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(store.lookup("abc").await.unwrap().is_none());
        assert!(!store.entries.lock().unwrap().contains_key("abc"));
    }
}
