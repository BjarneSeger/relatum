//! Persistence contract for short-lived, single-use state.
//!
//! Some flows need to stash a small value under a key for a brief window and read
//! it back exactly once — the OIDC authorization-code flow's pending state (PKCE
//! verifier bound to a `state`) and its single-use handoff codes are the motivating
//! case. Unlike [`SessionRepository`](crate::ports::session::SessionRepository),
//! which keeps a fixed store-level lifetime and supports repeated reads, this port
//! takes a **per-call TTL** (the pending and handoff windows differ) and exposes a
//! **`take`** that removes the entry as it returns it (so a value cannot be redeemed
//! twice, even across replicas when backed by a shared store).
//!
//! Implemented in `relatum-infra` by an in-memory adapter (single node) and a
//! Valkey/Redis adapter (shared across replicas). Holding this state in a shared
//! store is what lets a browser SSO login started on one replica complete on another.

use std::future::Future;
use std::time::Duration;

use crate::errors::DomainError;
use crate::ports::status::StatusBackend;

/// Stores short-lived values that are read back at most once.
///
/// The methods return `Send` futures (and `StatusBackend` makes implementors
/// `Send + Sync`) so an adapter can sit behind the API's shared axum state.
pub trait EphemeralStore: StatusBackend {
    /// Store `value` under `key`, expiring it `ttl` after it is written. Overwrites
    /// any existing value for `key`.
    fn put(
        &self,
        key: &str,
        value: &str,
        ttl: Duration,
    ) -> impl Future<Output = Result<(), DomainError>> + Send;

    /// Remove and return the value stored under `key`, or `None` if it is unknown or
    /// expired. The removal is atomic, so a value is handed to at most one caller.
    fn take(&self, key: &str) -> impl Future<Output = Result<Option<String>, DomainError>> + Send;
}
