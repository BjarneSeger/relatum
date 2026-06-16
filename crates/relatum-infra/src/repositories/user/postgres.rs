//! PostgreSQL-backed [`UserStorage`].
//!
//! The `User` aggregate is flattened into columns (see the migrations under
//! `migrations/`): `marker` carries the directory [`DirectoryMarker`], `department`
//! the locally-assigned [`DepartmentId`] (NULL while the user is inert). Compiled
//! only when the `postgres` feature is enabled.

use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::{DepartmentId, UserId};
use relatum_domain::models::users::{DirectoryMarker, User};
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use relatum_domain::ports::userstorage::UserStorage;
use sqlx::{PgPool, Row};

/// Persists users in PostgreSQL.
///
/// Holds a [`PgPool`], which is cheap to clone and internally multiplexed, so
/// the store is `Clone + Send + Sync` and fits behind shared async state.
#[derive(Clone)]
pub struct PostgresUserStore {
    pool: PgPool,
}

impl PostgresUserStore {
    /// Wrap an existing connection pool (see [`crate::db::connect_pg`]).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Split a [`User`] into the row columns: `(username, marker, department)`. Borrows
/// from the user, so callers bind the `&str`s directly.
fn user_columns(user: &User) -> (&str, &'static str, Option<&str>) {
    let marker = match user.marker() {
        DirectoryMarker::Instructor => "instructor",
        DirectoryMarker::Trainee => "trainee",
        DirectoryMarker::Regular => "regular",
    };
    (
        user.username(),
        marker,
        user.department().map(DepartmentId::as_str),
    )
}

/// Reassemble a [`User`] from a selected row. An unknown `marker` is a
/// [`DomainError::Backend`], since the table's `CHECK` and the writer should make it
/// impossible.
fn row_to_user(row: &sqlx::postgres::PgRow) -> Result<User, DomainError> {
    let id: String = row.get("id");
    let username: String = row.get("username");
    let marker_kind: String = row.get("marker");
    let department: Option<String> = row.get("department");

    let marker = match marker_kind.as_str() {
        "instructor" => DirectoryMarker::Instructor,
        "trainee" => DirectoryMarker::Trainee,
        "regular" => DirectoryMarker::Regular,
        other => {
            tracing::error!(user_id = %id, marker = %other, "invalid user row: unknown marker");
            return Err(DomainError::Backend(format!(
                "invalid user row {id}: unknown marker {other:?}"
            )));
        }
    };

    Ok(User::new(
        UserId::new(id),
        username,
        marker,
        department.map(DepartmentId::new),
    ))
}

impl UserStorage for PostgresUserStore {
    #[tracing::instrument(skip(self, user), fields(user_id = %user.id().as_str()), level = "debug")]
    async fn store(&self, user: User) -> Result<(), DomainError> {
        let (username, marker, department) = user_columns(&user);
        sqlx::query(
            "INSERT INTO users (id, username, marker, department) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (id) DO UPDATE SET \
                 username = EXCLUDED.username, \
                 marker = EXCLUDED.marker, \
                 department = EXCLUDED.department",
        )
        .bind(user.id().as_str())
        .bind(username)
        .bind(marker)
        .bind(department)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres store user failed");
            DomainError::Backend(format!("postgres store user failed: {e}"))
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self, user), fields(user_id = %user.as_str()), level = "debug")]
    async fn lookup(&self, user: &UserId) -> Result<Option<User>, DomainError> {
        let row = sqlx::query("SELECT id, username, marker, department FROM users WHERE id = $1")
            .bind(user.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "postgres lookup user failed");
                DomainError::Backend(format!("postgres lookup user failed: {e}"))
            })?;
        row.as_ref().map(row_to_user).transpose()
    }

    #[tracing::instrument(skip(self, user), fields(user_id = %user.as_str()), level = "debug")]
    async fn remove(&self, user: &UserId) -> Result<User, DomainError> {
        let row = sqlx::query(
            "DELETE FROM users WHERE id = $1 RETURNING id, username, marker, department",
        )
        .bind(user.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres remove user failed");
            DomainError::Backend(format!("postgres remove user failed: {e}"))
        })?;
        match row {
            Some(row) => row_to_user(&row),
            None => {
                tracing::debug!("user not found");
                Err(DomainError::NotFound(format!("user {}", user.as_str())))
            }
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn list_all(&self) -> Result<Vec<User>, DomainError> {
        let rows = sqlx::query("SELECT id, username, marker, department FROM users")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "postgres list_all users failed");
                DomainError::Backend(format!("postgres list_all users failed: {e}"))
            })?;
        rows.iter().map(row_to_user).collect()
    }
}

impl StatusBackend for PostgresUserStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => PortStatus::Healthy,
            Err(e) => {
                tracing::warn!(error = %e, "postgres user-store ping failed");
                PortStatus::Unhealthy {
                    reason: format!("postgres ping failed: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{connect_pg, run_migrations};

    /// End-to-end against a real database. Ignored by default (it needs a running
    /// PostgreSQL); run with:
    /// `DATABASE_URL=postgres://postgres:pw@127.0.0.1:5432/postgres \
    ///   cargo test -p relatum-infra -- --ignored`
    #[tokio::test]
    #[ignore = "requires a running PostgreSQL server"]
    async fn store_lookup_list_remove_roundtrip() {
        let url = std::env::var("DATABASE_URL")
            .expect("set DATABASE_URL to a running PostgreSQL for the ignored tests");
        let pool = connect_pg(&url).await.expect("connect to postgres");
        run_migrations(&pool).await.expect("run migrations");
        let store = PostgresUserStore::new(pool);

        assert!(matches!(store.get_status().await, PortStatus::Healthy));

        // A unique id keeps concurrent test runs from colliding.
        let id = format!("test-{}", uuid::Uuid::new_v4().simple());
        let user = User::new(
            UserId::new(&id),
            "alice",
            DirectoryMarker::Regular,
            Some(DepartmentId::new("blue")),
        );
        store.store(user).await.unwrap();

        let found = store.lookup(&UserId::new(&id)).await.unwrap().unwrap();
        assert_eq!(found.username(), "alice");
        assert_eq!(*found.marker(), DirectoryMarker::Regular);
        assert_eq!(found.department().unwrap().as_str(), "blue");
        assert!(
            store
                .list_all()
                .await
                .unwrap()
                .iter()
                .any(|u| u.id().as_str() == id)
        );

        let removed = store.remove(&UserId::new(&id)).await.unwrap();
        assert_eq!(removed.id().as_str(), id);
        assert!(store.lookup(&UserId::new(&id)).await.unwrap().is_none());
        assert!(matches!(
            store.remove(&UserId::new(&id)).await,
            Err(DomainError::NotFound(_))
        ));
    }
}
