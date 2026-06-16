//! User handling
//!
//! A user's identity, login name, and directory marker come from the LDAP
//! directory and are refreshed by the periodic
//! [sync](crate::services::sync::DirectorySync). Their **department** is local
//! relatum state: an admin assigns it (see
//! [`UserAdmin`](crate::services::admin::UserAdmin)) and the sync preserves it.
//!
//! The marker and the department together decide the user's *effective*
//! [`Role`]: a user in the instructor group is an instructor, a user in the
//! trainee group is a trainee, and any other ("regular") user becomes a **signer**
//! once they have a department. A user with no department has no role at all — they
//! are inert until assigned one.

use crate::models::ids::{DepartmentId, UserId};

/// Holds information on a single user.
#[derive(Debug, Clone)]
pub struct User {
    /// Stable identity of the user. See [`UserId`].
    id: UserId,
    /// The login name as recorded in the directory.
    username: String,
    /// What the directory groups mark this user as. See [`DirectoryMarker`].
    marker: DirectoryMarker,
    /// The department this user is manually assigned to, if any. `None` means the
    /// user is inert (has no effective [`Role`]).
    department: Option<DepartmentId>,
}

impl User {
    /// Assemble a user from its identity, login name, directory marker, and
    /// (optional) department assignment.
    pub fn new(
        id: UserId,
        username: impl Into<String>,
        marker: DirectoryMarker,
        department: Option<DepartmentId>,
    ) -> Self {
        Self {
            id,
            username: username.into(),
            marker,
            department,
        }
    }

    /// The user's stable identity.
    pub fn id(&self) -> &UserId {
        &self.id
    }

    /// The user's directory login name.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// What the directory marks this user as, independent of any department.
    pub fn marker(&self) -> &DirectoryMarker {
        &self.marker
    }

    /// The department this user is assigned to, or `None` if unassigned.
    pub fn department(&self) -> Option<&DepartmentId> {
        self.department.as_ref()
    }

    /// The user's *effective* role, derived from their marker and department.
    ///
    /// Returns `None` while the user has no department: such a user is inert and
    /// can neither author, sign, nor review anything.
    pub fn role(&self) -> Option<Role> {
        let department = self.department.clone()?;
        Some(match self.marker {
            DirectoryMarker::Instructor => Role::Instructor { department },
            DirectoryMarker::Trainee => Role::Trainee { department },
            DirectoryMarker::Regular => Role::Signer { department },
        })
    }

    /// Return a copy of this user assigned to `department`.
    pub fn with_department(&self, department: DepartmentId) -> Self {
        Self {
            department: Some(department),
            ..self.clone()
        }
    }

    /// Return a copy of this user with no department (rendering them inert).
    pub fn without_department(&self) -> Self {
        Self {
            department: None,
            ..self.clone()
        }
    }
}

/// What the directory's group membership marks a user as.
///
/// Set by the [sync](crate::services::sync::DirectorySync) from LDAP group
/// membership: members of the instructor group are [`Instructor`s](Self::Instructor),
/// members of the trainee group are [`Trainee`s](Self::Trainee), and everyone else
/// is [`Regular`](Self::Regular) — a user who becomes a *signer* once given a
/// department.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryMarker {
    /// Member of the instructor group.
    Instructor,
    /// Member of the trainee group.
    Trainee,
    /// In neither group; a signer once assigned a department.
    Regular,
}

/// A user's *effective* role, derived from their [`DirectoryMarker`] and assigned
/// [`DepartmentId`].
///
/// Every variant carries a department. For a [`Trainee`](Self::Trainee) it is the
/// queue their reports flow into; for a [`Signer`](Self::Signer) it is the queue
/// they may sign in. An [`Instructor`](Self::Instructor) also carries one for
/// bookkeeping, but instructor access is *global* — an instructor may read every
/// department's queue regardless of their own.
#[derive(Debug, Clone)]
pub enum Role {
    Instructor { department: DepartmentId },
    Trainee { department: DepartmentId },
    Signer { department: DepartmentId },
}

impl Role {
    /// The department this role is scoped to.
    pub fn department(&self) -> &DepartmentId {
        match self {
            Role::Instructor { department }
            | Role::Trainee { department }
            | Role::Signer { department } => department,
        }
    }

    /// Whether this user is a trainee (the role that authors reports).
    pub fn is_trainee(&self) -> bool {
        matches!(self, Role::Trainee { .. })
    }

    /// Whether this user is an instructor (read-only, global access to queues).
    pub fn is_instructor(&self) -> bool {
        matches!(self, Role::Instructor { .. })
    }

    /// Whether this user is a signer (the role that signs and rejects reports).
    pub fn is_signer(&self) -> bool {
        matches!(self, Role::Signer { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(marker: DirectoryMarker, department: Option<&str>) -> User {
        User::new(
            UserId::new("u"),
            "u",
            marker,
            department.map(DepartmentId::new),
        )
    }

    #[test]
    fn a_user_without_a_department_has_no_role() {
        assert!(user(DirectoryMarker::Instructor, None).role().is_none());
        assert!(user(DirectoryMarker::Trainee, None).role().is_none());
        assert!(user(DirectoryMarker::Regular, None).role().is_none());
    }

    #[test]
    fn marker_and_department_derive_the_role() {
        assert!(matches!(
            user(DirectoryMarker::Instructor, Some("blue")).role(),
            Some(Role::Instructor { .. })
        ));
        assert!(matches!(
            user(DirectoryMarker::Trainee, Some("blue")).role(),
            Some(Role::Trainee { .. })
        ));
        // A regular directory user becomes a signer once given a department.
        assert!(matches!(
            user(DirectoryMarker::Regular, Some("blue")).role(),
            Some(Role::Signer { .. })
        ));
    }

    #[test]
    fn assigning_and_clearing_a_department_toggles_activity() {
        let inert = user(DirectoryMarker::Regular, None);
        let active = inert.with_department(DepartmentId::new("blue"));
        assert!(active.role().is_some());
        assert_eq!(active.department().unwrap().as_str(), "blue");
        assert!(active.without_department().role().is_none());
    }
}
