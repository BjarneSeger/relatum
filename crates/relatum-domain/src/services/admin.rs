//! Manual user administration.
//!
//! The directory owns *who* a user is and *which group* they belong to; the one
//! thing it does **not** own is the user's department. [`UserAdmin`] is the single
//! place that manual assignment happens. It holds the
//! [`DepartmentRegistry`](crate::models::department::DepartmentRegistry) built at
//! startup and refuses to assign a department the instance does not recognise.
//!
//! Assigning a department to a regular directory user is what turns them into a
//! signer; clearing it renders any user inert again.

use crate::errors::DomainError;
use crate::models::department::DepartmentRegistry;
use crate::models::ids::{DepartmentId, UserId};
use crate::ports::userstorage::UserStorage;

/// Assigns and clears users' departments, validating against the configured set.
#[derive(Debug, Clone)]
pub struct UserAdmin<U> {
    users: U,
    departments: DepartmentRegistry,
}

impl<U> UserAdmin<U>
where
    U: UserStorage,
{
    /// Wire the admin service to user storage and the configured departments.
    pub fn new(users: U, departments: DepartmentRegistry) -> Self {
        Self { users, departments }
    }

    /// Assign `user` to `department`, making them active.
    ///
    /// Fails with [`DomainError::Invalid`] if `department` is not one of the
    /// configured departments, or [`DomainError::NotFound`] if the user does not
    /// exist.
    pub async fn assign_department(
        &self,
        user: &UserId,
        department: DepartmentId,
    ) -> Result<(), DomainError> {
        if !self.departments.contains(&department) {
            return Err(DomainError::Invalid(format!(
                "unknown department {}",
                department.as_str()
            )));
        }
        let existing = self.load(user).await?;
        self.users.store(existing.with_department(department)).await
    }

    /// Clear `user`'s department, rendering them inert. Fails with
    /// [`DomainError::NotFound`] if the user does not exist.
    pub async fn clear_department(&self, user: &UserId) -> Result<(), DomainError> {
        let existing = self.load(user).await?;
        self.users.store(existing.without_department()).await
    }

    /// Every user the instance knows about.
    ///
    /// Backs the instructor's administration view: it is the only way a frontend
    /// can present users *by name* so a department can be assigned without anyone
    /// having to type a raw [`UserId`]. Carries no authorization of its own — the
    /// caller (the API handler) gates it on the actor being an instructor, exactly
    /// as it does for [`assign_department`](Self::assign_department).
    pub async fn list_users(&self) -> Result<Vec<crate::models::users::User>, DomainError> {
        self.users.list_all().await
    }

    async fn load(&self, user: &UserId) -> Result<crate::models::users::User, DomainError> {
        self.users
            .lookup(user)
            .await?
            .ok_or_else(|| DomainError::NotFound(format!("user {}", user.as_str())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::users::{DirectoryMarker, User};
    use crate::testing::{InMemoryUsers, block_on};

    fn admin(users: InMemoryUsers) -> UserAdmin<InMemoryUsers> {
        UserAdmin::new(
            users,
            DepartmentRegistry::new([DepartmentId::new("blue"), DepartmentId::new("red")]),
        )
    }

    fn regular_user(users: &InMemoryUsers, id: &str) {
        block_on(users.store(User::new(
            UserId::new(id),
            id,
            DirectoryMarker::Regular,
            None,
        )))
        .unwrap();
    }

    #[test]
    fn assigning_a_known_department_activates_the_user_as_a_signer() {
        let users = InMemoryUsers::default();
        regular_user(&users, "alice");

        block_on(admin(users.clone()).assign_department(&UserId::new("alice"), DepartmentId::new("blue")))
            .unwrap();

        let alice = block_on(users.lookup(&UserId::new("alice"))).unwrap().unwrap();
        assert_eq!(alice.department().unwrap().as_str(), "blue");
        assert!(alice.role().unwrap().is_signer());
    }

    #[test]
    fn assigning_an_unknown_department_is_rejected() {
        let users = InMemoryUsers::default();
        regular_user(&users, "alice");

        let err = block_on(
            admin(users).assign_department(&UserId::new("alice"), DepartmentId::new("green")),
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)));
    }

    #[test]
    fn assigning_to_an_unknown_user_is_not_found() {
        let err = block_on(
            admin(InMemoryUsers::default())
                .assign_department(&UserId::new("ghost"), DepartmentId::new("blue")),
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::NotFound(_)));
    }

    #[test]
    fn list_users_returns_every_stored_user() {
        let users = InMemoryUsers::default();
        regular_user(&users, "alice");
        regular_user(&users, "bob");

        let listed = block_on(admin(users).list_users()).unwrap();

        let mut names: Vec<String> = listed
            .iter()
            .map(|u| u.username().to_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["alice".to_owned(), "bob".to_owned()]);
    }

    #[test]
    fn clearing_a_department_renders_the_user_inert() {
        let users = InMemoryUsers::default();
        regular_user(&users, "alice");
        block_on(admin(users.clone()).assign_department(&UserId::new("alice"), DepartmentId::new("blue")))
            .unwrap();

        block_on(admin(users.clone()).clear_department(&UserId::new("alice"))).unwrap();

        let alice = block_on(users.lookup(&UserId::new("alice"))).unwrap().unwrap();
        assert!(alice.department().is_none());
        assert!(alice.role().is_none());
    }
}
