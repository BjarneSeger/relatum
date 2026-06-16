//! Directory-sync use-case.
//!
//! [`DirectorySync`] reconciles the external directory (LDAP) into local
//! [`UserStorage`](crate::ports::userstorage::UserStorage). It is what replaces
//! password registration: users are *never* created out of band; they appear and
//! disappear as the directory says.
//!
//! Reconciliation rule, applied on each run:
//! - an entry not seen locally is **created** (with no department — inert until an
//!   admin assigns one);
//! - an entry already present has its `username`/`marker` **refreshed**, while its
//!   locally-assigned **department is preserved** (the directory is not
//!   authoritative for departments);
//! - a local user absent from the directory is **removed**.
//!
//! Periodic scheduling lives outside the domain (in `relatum-server`); this
//! service only performs one reconciliation pass when asked.

use std::collections::HashSet;

use crate::errors::DomainError;
use crate::models::users::User;
use crate::ports::directory::{DirectoryEntry, DirectorySource};
use crate::ports::userstorage::UserStorage;

/// The outcome of one [`DirectorySync::sync`] pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncSummary {
    /// Users created because they were new in the directory.
    pub added: usize,
    /// Existing users whose marker or username was refreshed.
    pub updated: usize,
    /// Local users removed because they vanished from the directory.
    pub removed: usize,
}

/// Reconciles directory entries into user storage.
#[derive(Debug, Clone)]
pub struct DirectorySync<D, U> {
    directory: D,
    users: U,
}

impl<D, U> DirectorySync<D, U>
where
    D: DirectorySource,
    U: UserStorage,
{
    /// Wire the sync to its directory-source and user-storage ports.
    pub fn new(directory: D, users: U) -> Self {
        Self { directory, users }
    }

    /// Pull every directory entry and reconcile it into storage, returning a
    /// summary of what changed. Locally-assigned departments are preserved.
    pub async fn sync(&self) -> Result<SyncSummary, DomainError> {
        let entries = self.directory.list_entries().await?;
        let mut summary = SyncSummary::default();

        let mut seen: HashSet<String> = HashSet::with_capacity(entries.len());
        for entry in entries {
            seen.insert(entry.id.as_str().to_owned());
            match self.users.lookup(&entry.id).await? {
                Some(existing) => {
                    if let Some(merged) = merge(&existing, &entry) {
                        self.users.store(merged).await?;
                        summary.updated += 1;
                    }
                }
                None => {
                    self.users.store(provision(entry)).await?;
                    summary.added += 1;
                }
            }
        }

        // Prune local users the directory no longer lists.
        for user in self.users.list_all().await? {
            if !seen.contains(user.id().as_str()) {
                self.users.remove(user.id()).await?;
                summary.removed += 1;
            }
        }

        Ok(summary)
    }
}

/// A brand-new local user for a directory entry: marker from the directory, no
/// department yet (inert until an admin assigns one).
fn provision(entry: DirectoryEntry) -> User {
    User::new(entry.id, entry.username, entry.marker, None)
}

/// Refresh an existing user from a directory entry, **keeping** their department.
/// Returns `None` when nothing changed, so storage is only written when needed.
fn merge(existing: &User, entry: &DirectoryEntry) -> Option<User> {
    if existing.username() == entry.username && *existing.marker() == entry.marker {
        return None;
    }
    Some(User::new(
        entry.id.clone(),
        entry.username.clone(),
        entry.marker,
        existing.department().cloned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ids::{DepartmentId, UserId};
    use crate::models::users::DirectoryMarker;
    use crate::testing::{InMemoryDirectory, InMemoryUsers, block_on};

    fn entry(id: &str, marker: DirectoryMarker) -> DirectoryEntry {
        DirectoryEntry {
            id: UserId::new(id),
            username: id.to_owned(),
            marker,
        }
    }

    fn sync(directory: InMemoryDirectory, users: InMemoryUsers) -> DirectorySync<InMemoryDirectory, InMemoryUsers> {
        DirectorySync::new(directory, users)
    }

    #[test]
    fn new_entries_are_provisioned_without_a_department() {
        let dir = InMemoryDirectory::default();
        dir.set([entry("alice", DirectoryMarker::Trainee)]);
        let users = InMemoryUsers::default();

        let summary = block_on(sync(dir, users.clone()).sync()).unwrap();

        assert_eq!(summary.added, 1);
        let alice = block_on(users.lookup(&UserId::new("alice"))).unwrap().unwrap();
        assert!(alice.department().is_none());
        assert_eq!(*alice.marker(), DirectoryMarker::Trainee);
    }

    #[test]
    fn sync_preserves_a_manually_assigned_department() {
        let users = InMemoryUsers::default();
        // alice was synced earlier and then assigned to "blue".
        block_on(users.store(User::new(
            UserId::new("alice"),
            "alice",
            DirectoryMarker::Regular,
            Some(DepartmentId::new("blue")),
        )))
        .unwrap();

        // The directory now marks her a trainee; a re-sync must keep "blue".
        let dir = InMemoryDirectory::default();
        dir.set([entry("alice", DirectoryMarker::Trainee)]);

        let summary = block_on(sync(dir, users.clone()).sync()).unwrap();

        assert_eq!(summary.updated, 1);
        let alice = block_on(users.lookup(&UserId::new("alice"))).unwrap().unwrap();
        assert_eq!(*alice.marker(), DirectoryMarker::Trainee);
        assert_eq!(alice.department().unwrap().as_str(), "blue");
    }

    #[test]
    fn unchanged_entries_are_not_rewritten() {
        let users = InMemoryUsers::default();
        block_on(users.store(User::new(
            UserId::new("alice"),
            "alice",
            DirectoryMarker::Trainee,
            None,
        )))
        .unwrap();
        let dir = InMemoryDirectory::default();
        dir.set([entry("alice", DirectoryMarker::Trainee)]);

        let summary = block_on(sync(dir, users).sync()).unwrap();
        assert_eq!(summary, SyncSummary { added: 0, updated: 0, removed: 0 });
    }

    #[test]
    fn users_absent_from_the_directory_are_pruned() {
        let users = InMemoryUsers::default();
        block_on(users.store(User::new(
            UserId::new("gone"),
            "gone",
            DirectoryMarker::Trainee,
            Some(DepartmentId::new("blue")),
        )))
        .unwrap();
        let dir = InMemoryDirectory::default();
        dir.set([entry("alice", DirectoryMarker::Trainee)]);

        let summary = block_on(sync(dir, users.clone()).sync()).unwrap();

        assert_eq!(summary.added, 1);
        assert_eq!(summary.removed, 1);
        assert!(block_on(users.lookup(&UserId::new("gone"))).unwrap().is_none());
    }
}
