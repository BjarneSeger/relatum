//! Reading the user directory (LDAP).
//!
//! Relatum no longer manages passwords or out-of-band registration: the set of
//! users — and which group each belongs to — is owned by an external directory
//! (LDAP) and pulled in periodically. This port is the *read* side of that: it
//! lists every directory entry so the [sync](crate::services::sync::DirectorySync)
//! can reconcile them into [`UserStorage`](crate::ports::userstorage::UserStorage),
//! creating new users, refreshing markers, and pruning ones that have disappeared
//! — all while preserving each user's locally-assigned department.

use std::future::Future;

use crate::DomainError;
use crate::models::ids::UserId;
use crate::models::users::DirectoryMarker;
use crate::ports::status::StatusBackend;

/// One user as seen in the external directory.
///
/// Carries only what the directory is authoritative for: the stable id, the login
/// name, and the group-derived [`DirectoryMarker`]. The department is *not* here —
/// it is local relatum state the sync deliberately leaves untouched.
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    /// Stable identity, matching the subject an SSO token will attest.
    pub id: UserId,
    /// The login name recorded for the user.
    pub username: String,
    /// What the directory's group membership marks the user as.
    pub marker: DirectoryMarker,
}

/// Lists the users an external directory (e.g. LDAP) knows about.
pub trait DirectorySource: StatusBackend {
    /// Fetch every user the directory currently contains.
    fn list_entries(
        &self,
    ) -> impl Future<Output = Result<Vec<DirectoryEntry>, DomainError>> + Send;
}
