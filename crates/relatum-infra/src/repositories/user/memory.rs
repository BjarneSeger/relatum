//! In-memory [`UserStorage`] backed by a `HashMap`, keyed by [`UserId`].
//!
//! Self-contained: it needs no external service, so it is the natural default
//! for development, single-node deployments and tests. Cheap to clone — clones
//! share one `Arc<Mutex<…>>` — so the store can be handed to several handlers.
//! It is `Send + Sync`: the mutex guard is never held across an `.await`, so the
//! in-trait async futures stay `Send`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::UserId;
use relatum_domain::models::users::User;
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use relatum_domain::ports::userstorage::UserStorage;

/// In-memory [`UserStorage`], keyed by [`UserId`].
#[derive(Clone, Default)]
pub struct InMemoryUsers {
    users: Arc<Mutex<HashMap<String, User>>>,
}

impl InMemoryUsers {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl UserStorage for InMemoryUsers {
    async fn store(&self, user: User) -> Result<(), DomainError> {
        self.users
            .lock()
            .unwrap()
            .insert(user.id().as_str().to_owned(), user);
        Ok(())
    }

    async fn lookup(&self, user: &UserId) -> Result<Option<User>, DomainError> {
        Ok(self.users.lock().unwrap().get(user.as_str()).cloned())
    }

    async fn remove(&self, user: &UserId) -> Result<User, DomainError> {
        self.users
            .lock()
            .unwrap()
            .remove(user.as_str())
            .ok_or_else(|| DomainError::NotFound(format!("user {}", user.as_str())))
    }

    async fn list_all(&self) -> Result<Vec<User>, DomainError> {
        Ok(self.users.lock().unwrap().values().cloned().collect())
    }
}

impl StatusBackend for InMemoryUsers {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relatum_domain::models::ids::DepartmentId;
    use relatum_domain::models::users::DirectoryMarker;

    fn user(id: &str) -> User {
        User::new(
            UserId::new(id),
            id,
            DirectoryMarker::Instructor,
            Some(DepartmentId::new("blue")),
        )
    }

    #[tokio::test]
    async fn stores_and_looks_up_a_user() {
        let store = InMemoryUsers::new();
        store.store(user("alice")).await.unwrap();

        let found = store.lookup(&UserId::new("alice")).await.unwrap();
        assert_eq!(
            found.map(|u| u.id().as_str().to_owned()),
            Some("alice".to_owned())
        );
    }

    #[tokio::test]
    async fn unknown_user_is_none() {
        let store = InMemoryUsers::new();
        assert!(
            store
                .lookup(&UserId::new("nobody"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn store_overwrites_existing_user() {
        let store = InMemoryUsers::new();
        store.store(user("alice")).await.unwrap();
        store
            .store(User::new(
                UserId::new("alice"),
                "alice",
                DirectoryMarker::Trainee,
                Some(DepartmentId::new("red")),
            ))
            .await
            .unwrap();

        let found = store.lookup(&UserId::new("alice")).await.unwrap().unwrap();
        assert_eq!(*found.marker(), DirectoryMarker::Trainee);
        assert_eq!(found.department().unwrap().as_str(), "red");
    }

    #[tokio::test]
    async fn remove_returns_the_user_then_absent() {
        let store = InMemoryUsers::new();
        store.store(user("alice")).await.unwrap();

        let removed = store.remove(&UserId::new("alice")).await.unwrap();
        assert_eq!(removed.id().as_str(), "alice");
        assert!(store.lookup(&UserId::new("alice")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_absent_user_is_not_found() {
        let store = InMemoryUsers::new();
        assert!(matches!(
            store.remove(&UserId::new("ghost")).await,
            Err(DomainError::NotFound(_))
        ));
    }
}
