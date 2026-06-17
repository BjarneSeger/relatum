//! In-memory [`SignatureStorage`] backed by a `HashMap`, keyed by [`UserId`].
//!
//! Self-contained: it needs no external service, so it is the natural default for
//! development, single-node deployments and tests. Cheap to clone — clones share
//! one `Arc<Mutex<…>>`. `Send + Sync`: the mutex guard is never held across an
//! `.await`, so the in-trait async futures stay `Send`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::UserId;
use relatum_domain::models::signature::{Signature, StoredSignature};
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::status::{PortStatus, StatusBackend};

/// In-memory [`SignatureStorage`], keyed by [`UserId`].
#[derive(Clone, Default)]
pub struct InMemorySignatures {
    signatures: Arc<Mutex<HashMap<String, StoredSignature>>>,
}

impl InMemorySignatures {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SignatureStorage for InMemorySignatures {
    async fn set(
        &self,
        user: &UserId,
        signature: Signature,
        at: Timestamp,
    ) -> Result<(), DomainError> {
        self.signatures.lock().unwrap().insert(
            user.as_str().to_owned(),
            StoredSignature {
                signature,
                updated_at: at,
            },
        );
        Ok(())
    }

    async fn get(&self, user: &UserId) -> Result<Option<StoredSignature>, DomainError> {
        Ok(self.signatures.lock().unwrap().get(user.as_str()).cloned())
    }
}

impl StatusBackend for InMemorySignatures {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relatum_domain::models::signature::SignatureFormat;

    fn sig(last: u8) -> Signature {
        Signature::new(
            SignatureFormat::Png,
            vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, last],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let store = InMemorySignatures::new();
        store
            .set(&UserId::new("u"), sig(1), Timestamp::now())
            .await
            .unwrap();

        let got = store.get(&UserId::new("u")).await.unwrap().unwrap();
        assert_eq!(got.signature.bytes(), sig(1).bytes());
    }

    #[tokio::test]
    async fn unknown_user_is_none() {
        let store = InMemorySignatures::new();
        assert!(store.get(&UserId::new("nobody")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_replaces_the_previous_signature() {
        let store = InMemorySignatures::new();
        store
            .set(&UserId::new("u"), sig(1), Timestamp::now())
            .await
            .unwrap();
        store
            .set(&UserId::new("u"), sig(2), Timestamp::now())
            .await
            .unwrap();

        let got = store.get(&UserId::new("u")).await.unwrap().unwrap();
        assert_eq!(got.signature.bytes(), sig(2).bytes());
    }
}
