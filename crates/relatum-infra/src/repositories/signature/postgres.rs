//! PostgreSQL-backed [`SignatureStorage`].
//!
//! A user's signature is a single row in the `signatures` table (see
//! `migrations/0002_add_signatures.sql`): the raw image bytes in a `BYTEA` `image`
//! column — the first binary payload in the schema — plus a `format` discriminant
//! and an `updated_at` RFC 3339 text timestamp, mirroring how the rest of the schema
//! stores [`jiff::Timestamp`] values. Compiled only when the `postgres` feature is
//! enabled.

use jiff::Timestamp;
use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::UserId;
use relatum_domain::models::signature::{Signature, SignatureFormat, StoredSignature};
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use sqlx::{PgPool, Row};

/// Persists user signatures in PostgreSQL.
///
/// Holds a [`PgPool`], which is cheap to clone and internally multiplexed, so the
/// store is `Clone + Send + Sync` and fits behind shared async state.
#[derive(Clone)]
pub struct PostgresSignatureStore {
    pool: PgPool,
}

impl PostgresSignatureStore {
    /// Wrap an existing connection pool (see [`crate::db::connect_pg`]).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Reassemble a [`StoredSignature`] from a selected row. A row that does not parse
/// (unknown `format`, unparseable `updated_at`) is a [`DomainError::Backend`], since
/// the table's `CHECK` and the writer should make it impossible.
fn row_to_signature(row: &sqlx::postgres::PgRow) -> Result<StoredSignature, DomainError> {
    let user_id: String = row.get("user_id");
    let format: String = row.get("format");
    let image: Vec<u8> = row.get("image");
    let updated_at: String = row.get("updated_at");

    let format = SignatureFormat::parse(&format)
        .map_err(|e| DomainError::Backend(format!("invalid signature row {user_id}: {e}")))?;
    let updated_at = updated_at.parse::<Timestamp>().map_err(|e| {
        DomainError::Backend(format!(
            "invalid signature row {user_id}: bad timestamp: {e}"
        ))
    })?;
    Ok(StoredSignature {
        signature: Signature::from_stored(format, image),
        updated_at,
    })
}

impl SignatureStorage for PostgresSignatureStore {
    #[tracing::instrument(skip(self, signature), fields(user_id = %user.as_str()), level = "debug")]
    async fn set(
        &self,
        user: &UserId,
        signature: Signature,
        at: Timestamp,
    ) -> Result<(), DomainError> {
        sqlx::query(
            "INSERT INTO signatures (user_id, format, image, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (user_id) DO UPDATE SET \
                 format = EXCLUDED.format, \
                 image = EXCLUDED.image, \
                 updated_at = EXCLUDED.updated_at",
        )
        .bind(user.as_str())
        .bind(signature.format().as_str())
        .bind(signature.bytes())
        .bind(at.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres set signature failed");
            DomainError::Backend(format!("postgres set signature failed: {e}"))
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(user_id = %user.as_str()), level = "debug")]
    async fn get(&self, user: &UserId) -> Result<Option<StoredSignature>, DomainError> {
        let row = sqlx::query(
            "SELECT user_id, format, image, updated_at FROM signatures WHERE user_id = $1",
        )
        .bind(user.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres get signature failed");
            DomainError::Backend(format!("postgres get signature failed: {e}"))
        })?;
        row.as_ref().map(row_to_signature).transpose()
    }
}

impl StatusBackend for PostgresSignatureStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => PortStatus::Healthy,
            Err(e) => {
                tracing::warn!(error = %e, "postgres signature-store ping failed");
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
    async fn set_get_replace_roundtrip() {
        let url = std::env::var("DATABASE_URL")
            .expect("set DATABASE_URL to a running PostgreSQL for the ignored tests");
        let pool = connect_pg(&url).await.expect("connect to postgres");
        run_migrations(&pool).await.expect("run migrations");
        let store = PostgresSignatureStore::new(pool);

        assert!(matches!(store.get_status().await, PortStatus::Healthy));

        // A unique id keeps concurrent test runs from colliding.
        let user = UserId::new(format!("user-{}", uuid::Uuid::new_v4().simple()));
        assert!(store.get(&user).await.unwrap().is_none());

        let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02];
        let at = Timestamp::now();
        store
            .set(
                &user,
                Signature::new(SignatureFormat::Png, png.clone()).unwrap(),
                at,
            )
            .await
            .unwrap();

        let got = store.get(&user).await.unwrap().unwrap();
        assert_eq!(got.signature.format(), SignatureFormat::Png);
        assert_eq!(got.signature.bytes(), png.as_slice());
        assert_eq!(got.updated_at, at);

        // A second set replaces the image in place.
        let png2 = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFE];
        store
            .set(
                &user,
                Signature::new(SignatureFormat::Png, png2.clone()).unwrap(),
                Timestamp::now(),
            )
            .await
            .unwrap();
        let got = store.get(&user).await.unwrap().unwrap();
        assert_eq!(got.signature.bytes(), png2.as_slice());
    }
}
