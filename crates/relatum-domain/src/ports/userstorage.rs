//! Storage for users

use std::future::Future;

use crate::{
    DomainError,
    models::{ids::UserId, users::User},
    ports::status::StatusBackend,
};

/// Allows storing an retrieving User to a persistent backend, e.g. a relational
/// database.
///
/// `StatusBackend` already requires `Send + Sync`; the methods return `Send`
/// futures so the user services can run behind the shared axum state.
pub trait UserStorage: StatusBackend {
    /// Store a user persistently.
    fn store(&self, user: User) -> impl Future<Output = Result<(), DomainError>> + Send;
    /// Get a user from persistent storage.
    ///
    /// Returns an error if backend operations failed or None if the user did not
    /// exist.
    fn lookup(
        &self,
        user: &UserId,
    ) -> impl Future<Output = Result<Option<User>, DomainError>> + Send;
    /// Remove a user from storage.
    fn remove(&self, user: &UserId) -> impl Future<Output = Result<User, DomainError>> + Send;

    /// Every user currently stored.
    ///
    /// Powers the directory [sync](crate::services::sync::DirectorySync), which
    /// needs the full set to prune users that have disappeared from the directory.
    fn list_all(&self) -> impl Future<Output = Result<Vec<User>, DomainError>> + Send;
}
