//! LDAP-backed [`DirectorySource`].
//!
//! The read side of the periodic user sync: it lists every user the LDAP directory
//! knows about, mapping group membership onto a
//! [`DirectoryMarker`](relatum_domain::models::users::DirectoryMarker) (instructor
//! group → `Instructor`, trainee group → `Trainee`, otherwise `Regular`). Compiled
//! only when the `ldap` feature is enabled.
//!
//! [`DirectorySource`]: relatum_domain::ports::directory::DirectorySource

use ldap3::{Ldap, LdapConnAsync, Scope, SearchEntry};
use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::UserId;
use relatum_domain::models::users::DirectoryMarker;
use relatum_domain::ports::directory::{DirectoryEntry, DirectorySource};
use relatum_domain::ports::status::{PortStatus, StatusBackend};

/// Configuration for connecting to and querying an LDAP directory.
#[derive(Debug, Clone, Default)]
pub struct LdapConfig {
    /// The LDAP server URL (e.g. `ldaps://ldap.example:636`).
    pub url: String,
    /// The bind DN for the service account used to search the directory. Empty
    /// means an anonymous bind.
    pub bind_dn: String,
    /// The password for [`bind_dn`](Self::bind_dn). Ignored for an anonymous bind.
    pub bind_password: String,
    /// The search base under which user entries live (e.g. `ou=people,dc=example`).
    pub user_base: String,
    /// The search filter selecting user entries (e.g. `(objectClass=person)`).
    pub user_filter: String,
    /// The entry attribute whose value becomes the [`UserId`] — it must match the
    /// subject an SSO token attests (e.g. `uid` or `entryUUID`).
    pub id_attr: String,
    /// The entry attribute whose value becomes the login name (e.g. `uid` or `cn`).
    pub username_attr: String,
    /// The membership attribute inspected for group markers (e.g. `memberOf`).
    pub group_attr: String,
    /// The instructor group whose members map to `DirectoryMarker::Instructor`.
    pub instructor_group: String,
    /// The trainee group whose members map to `DirectoryMarker::Trainee`.
    pub trainee_group: String,
}

/// A [`DirectorySource`] backed by an LDAP directory.
#[derive(Debug, Clone, Default)]
pub struct LdapDirectory {
    config: LdapConfig,
}

impl LdapDirectory {
    /// Build an adapter for the given LDAP configuration.
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    /// Open a connection and bind, returning the live handle. The connection's
    /// driver is spawned onto the tokio runtime and lives as long as the handle.
    // `self` holds the bind password, so it is skipped from the span fields.
    #[tracing::instrument(skip(self), level = "debug")]
    async fn connect(&self) -> Result<Ldap, DomainError> {
        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url).await.map_err(|e| {
            tracing::error!(error = %e, "ldap connect failed");
            DomainError::Backend(format!("ldap connect failed: {e}"))
        })?;
        ldap3::drive!(conn);

        if self.config.bind_dn.is_empty() {
            // Anonymous bind: no credentials presented.
            ldap.simple_bind("", "")
                .await
                .and_then(|r| r.success())
                .map_err(|e| {
                    tracing::error!(error = %e, "ldap anonymous bind failed");
                    DomainError::Backend(format!("ldap anonymous bind failed: {e}"))
                })?;
        } else {
            ldap.simple_bind(&self.config.bind_dn, &self.config.bind_password)
                .await
                .and_then(|r| r.success())
                .map_err(|e| {
                    tracing::error!(error = %e, "ldap bind failed");
                    DomainError::Backend(format!("ldap bind failed: {e}"))
                })?;
        }
        Ok(ldap)
    }

    /// Map a user entry's group-membership values onto a [`DirectoryMarker`].
    /// Instructor membership wins over trainee; absent both, the user is `Regular`.
    fn marker_for(&self, groups: &[String]) -> DirectoryMarker {
        if groups.iter().any(|g| g == &self.config.instructor_group) {
            DirectoryMarker::Instructor
        } else if groups.iter().any(|g| g == &self.config.trainee_group) {
            DirectoryMarker::Trainee
        } else {
            DirectoryMarker::Regular
        }
    }
}

impl DirectorySource for LdapDirectory {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn list_entries(&self) -> Result<Vec<DirectoryEntry>, DomainError> {
        let mut ldap = self.connect().await?;

        let attrs = vec![
            self.config.id_attr.as_str(),
            self.config.username_attr.as_str(),
            self.config.group_attr.as_str(),
        ];
        let (rows, _res) = ldap
            .search(
                &self.config.user_base,
                Scope::Subtree,
                &self.config.user_filter,
                attrs,
            )
            .await
            .and_then(|r| r.success())
            .map_err(|e| {
                tracing::error!(error = %e, "ldap search failed");
                DomainError::Backend(format!("ldap search failed: {e}"))
            })?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let entry = SearchEntry::construct(row);

            // The id and username attributes are required; an entry missing either
            // cannot be reconciled, so skip it rather than fail the whole sync.
            let Some(id) = entry
                .attrs
                .get(&self.config.id_attr)
                .and_then(|v| v.first())
            else {
                tracing::warn!(dn = %entry.dn, attr = %self.config.id_attr, "ldap entry missing id attribute, skipping");
                continue;
            };
            let Some(username) = entry
                .attrs
                .get(&self.config.username_attr)
                .and_then(|v| v.first())
            else {
                tracing::warn!(dn = %entry.dn, attr = %self.config.username_attr, "ldap entry missing username attribute, skipping");
                continue;
            };

            let groups = entry.attrs.get(&self.config.group_attr);
            let marker = self.marker_for(groups.map(Vec::as_slice).unwrap_or(&[]));

            entries.push(DirectoryEntry {
                id: UserId::new(id.clone()),
                username: username.clone(),
                marker,
            });
        }

        let _ = ldap.unbind().await;
        tracing::debug!(count = entries.len(), "ldap entries listed");
        Ok(entries)
    }
}

impl StatusBackend for LdapDirectory {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        if self.config.url.is_empty() {
            return PortStatus::NotConnected;
        }
        match self.connect().await {
            Ok(mut ldap) => {
                let _ = ldap.unbind().await;
                PortStatus::Healthy
            }
            Err(e) => {
                tracing::warn!(error = %e, "ldap directory unreachable");
                PortStatus::Unhealthy {
                    reason: e.to_string(),
                }
            }
        }
    }
}
