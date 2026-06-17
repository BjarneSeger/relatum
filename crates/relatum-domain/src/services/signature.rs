//! Self-service signature registration.
//!
//! [`SignatureService`] is the use-case behind a user setting (or replacing) the
//! signature image a future PDF export stamps onto their reports. Like the other
//! services it is **auth-free**: *who* may register a signature — trainees and
//! signers, never read-only instructors — is decided one layer out, in the API
//! handler that binds the action to the authenticated caller. This service owns
//! only the clock (it stamps each save with `Timestamp::now()`) and the storage
//! call.

use crate::errors::DomainError;
use crate::models::ids::UserId;
use crate::models::signature::{Signature, StoredSignature};
use crate::ports::signaturestorage::SignatureStorage;
use jiff::Timestamp;

/// Sets and reads users' signatures.
#[derive(Debug, Clone)]
pub struct SignatureService<S> {
    signatures: S,
}

impl<S> SignatureService<S>
where
    S: SignatureStorage,
{
    /// Wire the service to signature storage.
    pub fn new(signatures: S) -> Self {
        Self { signatures }
    }

    /// Set (or replace) `user`'s signature, stamping it with the current time.
    pub async fn set(&self, user: &UserId, signature: Signature) -> Result<(), DomainError> {
        self.signatures.set(user, signature, Timestamp::now()).await
    }

    /// Read `user`'s signature, or `None` if they have not registered one.
    pub async fn get(&self, user: &UserId) -> Result<Option<StoredSignature>, DomainError> {
        self.signatures.get(user).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::signature::SignatureFormat;
    use crate::testing::{InMemorySignatures, block_on, dev_signature};

    fn service() -> SignatureService<InMemorySignatures> {
        SignatureService::new(InMemorySignatures::default())
    }

    #[test]
    fn get_is_none_until_a_signature_is_set() {
        let svc = service();
        assert!(block_on(svc.get(&UserId::new("u"))).unwrap().is_none());
    }

    #[test]
    fn set_then_get_round_trips() {
        let svc = service();
        block_on(svc.set(&UserId::new("u"), dev_signature())).unwrap();

        let stored = block_on(svc.get(&UserId::new("u"))).unwrap().unwrap();
        assert_eq!(stored.signature.format(), SignatureFormat::Png);
        assert_eq!(stored.signature.bytes(), dev_signature().bytes());
    }

    #[test]
    fn set_replaces_the_previous_signature() {
        let svc = service();
        block_on(svc.set(&UserId::new("u"), dev_signature())).unwrap();

        let replacement = Signature::new(
            SignatureFormat::Png,
            vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF],
        )
        .unwrap();
        block_on(svc.set(&UserId::new("u"), replacement.clone())).unwrap();

        let stored = block_on(svc.get(&UserId::new("u"))).unwrap().unwrap();
        assert_eq!(stored.signature.bytes(), replacement.bytes());
    }
}
