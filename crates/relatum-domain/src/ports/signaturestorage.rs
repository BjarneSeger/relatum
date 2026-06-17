//! Persistence contract for user signatures.
//!
//! A signature is a per-user asset: at most one per user, keyed by [`UserId`], set
//! once and replaced in place. It is kept apart from [`UserStorage`] —and the
//! directory-owned `users` row— on purpose, so the LDAP sync that rebuilds a
//! [`User`](crate::models::users::User) can never clobber it. Implemented in
//! `relatum-infra`.
//!
//! Mirrors the other storage ports: methods return `Send` futures so the services
//! can run behind the API's shared axum state (`StatusBackend` already requires
//! `Send + Sync`).
//!
//! [`UserStorage`]: crate::ports::userstorage::UserStorage

use std::future::Future;

use crate::DomainError;
use crate::models::ids::UserId;
use crate::models::signature::{Signature, StoredSignature};
use crate::ports::status::StatusBackend;
use jiff::Timestamp;

pub trait SignatureStorage: StatusBackend {
    /// Store `user`'s signature, inserting it or replacing the one already on file.
    /// `at` records when it was set.
    fn set(
        &self,
        user: &UserId,
        signature: Signature,
        at: Timestamp,
    ) -> impl Future<Output = Result<(), DomainError>> + Send;

    /// Load `user`'s signature, or `None` if they have not registered one.
    fn get(
        &self,
        user: &UserId,
    ) -> impl Future<Output = Result<Option<StoredSignature>, DomainError>> + Send;
}
