//! PostgreSQL-backed [`ReportStorage`].
//!
//! A [`Report`] now belongs to a `department` queue (instead of a chosen reviewer)
//! and its terminal state is `Signed { by }` rather than `Accepted`. The
//! [`ReviewStatus`] is flattened into a `status` discriminant plus its payload
//! columns (see `migrations/0001_create_users_and_reports.sql`): `status_at` holds
//! the transition timestamp, `signed_by` the signer, and `reject_reason` the
//! rejection text. Timestamps are stored as RFC 3339 text because the domain models
//! time with [`jiff::Timestamp`], which `sqlx` does not map natively. Compiled only
//! when the `postgres` feature is enabled.

use jiff::Timestamp;
use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::{DepartmentId, ReportId, UserId};
use relatum_domain::models::report::{Report, ReviewStatus};
use relatum_domain::models::week::IsoWeek;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use sqlx::{PgPool, Row};

/// Persists reports in PostgreSQL.
///
/// Holds a [`PgPool`], which is cheap to clone and internally multiplexed, so
/// the store is `Clone + Send + Sync` and fits behind shared async state.
#[derive(Clone)]
pub struct PostgresReportStore {
    pool: PgPool,
}

impl PostgresReportStore {
    /// Wrap an existing connection pool (see [`crate::db::connect_pg`]).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Split a [`ReviewStatus`] into the row columns: `(status, status_at, signed_by,
/// reject_reason)`. `status_at` is the timestamp formatted as RFC 3339 text.
fn status_columns(
    status: &ReviewStatus,
) -> (&'static str, Option<String>, Option<&str>, Option<&str>) {
    match status {
        ReviewStatus::Draft => ("draft", None, None, None),
        ReviewStatus::Submitted { at } => ("submitted", Some(at.to_string()), None, None),
        ReviewStatus::Signed { at, by } => {
            ("signed", Some(at.to_string()), Some(by.as_str()), None)
        }
        ReviewStatus::Rejected { at, reason } => {
            ("rejected", Some(at.to_string()), None, Some(reason.as_str()))
        }
    }
}

/// Rebuild a [`ReviewStatus`] from its columns. An internally inconsistent row
/// (e.g. `signed` with a NULL `signed_by`, or an unparseable timestamp) is a
/// [`DomainError::Backend`], since the table's `CHECK` and the writer should make
/// it impossible.
fn build_status(
    id: &str,
    kind: &str,
    status_at: Option<String>,
    signed_by: Option<String>,
    reject_reason: Option<String>,
) -> Result<ReviewStatus, DomainError> {
    let parse_at = |at: Option<String>| -> Result<Timestamp, DomainError> {
        at.ok_or_else(|| {
            DomainError::Backend(format!("invalid report row {id}: {kind} without status_at"))
        })?
        .parse::<Timestamp>()
        .map_err(|e| DomainError::Backend(format!("invalid report row {id}: bad timestamp: {e}")))
    };

    match kind {
        "draft" => Ok(ReviewStatus::Draft),
        "submitted" => Ok(ReviewStatus::Submitted {
            at: parse_at(status_at)?,
        }),
        "signed" => {
            let at = parse_at(status_at)?;
            let by = signed_by.ok_or_else(|| {
                DomainError::Backend(format!("invalid report row {id}: signed without signed_by"))
            })?;
            Ok(ReviewStatus::Signed {
                at,
                by: UserId::new(by),
            })
        }
        "rejected" => {
            let at = parse_at(status_at)?;
            let reason = reject_reason.ok_or_else(|| {
                DomainError::Backend(format!("invalid report row {id}: rejected without reason"))
            })?;
            Ok(ReviewStatus::Rejected { at, reason })
        }
        other => Err(DomainError::Backend(format!(
            "invalid report row {id}: unknown status {other:?}"
        ))),
    }
}

/// Reassemble a [`Report`] from a selected row.
fn row_to_report(row: &sqlx::postgres::PgRow) -> Result<Report, DomainError> {
    let id: String = row.get("id");
    let author: String = row.get("author");
    let department: String = row.get("department");
    let week: String = row.get("week");
    let content: String = row.get("content");
    let status_kind: String = row.get("status");
    let status_at: Option<String> = row.get("status_at");
    let signed_by: Option<String> = row.get("signed_by");
    let reject_reason: Option<String> = row.get("reject_reason");

    let week = week.parse::<IsoWeek>().map_err(|e| {
        tracing::error!(report_id = %id, error = %e, "invalid report row: bad week");
        DomainError::Backend(format!("invalid report row {id}: bad week: {e}"))
    })?;
    let status = build_status(&id, &status_kind, status_at, signed_by, reject_reason)
        .inspect_err(|e| tracing::error!(report_id = %id, error = %e, "invalid report row"))?;
    Ok(Report::from_parts(
        ReportId::new(id),
        UserId::new(author),
        DepartmentId::new(department),
        week,
        content,
        status,
    ))
}

impl ReportStorage for PostgresReportStore {
    #[tracing::instrument(
        skip(self, report),
        fields(report_id = %report.id().as_str(), author = %report.author().as_str()),
        level = "debug"
    )]
    async fn store(&self, report: &Report) -> Result<(), DomainError> {
        let (status, status_at, signed_by, reject_reason) = status_columns(report.status());
        sqlx::query(
            "INSERT INTO reports \
                 (id, author, department, week, content, status, status_at, signed_by, reject_reason) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (id) DO UPDATE SET \
                 author = EXCLUDED.author, \
                 department = EXCLUDED.department, \
                 week = EXCLUDED.week, \
                 content = EXCLUDED.content, \
                 status = EXCLUDED.status, \
                 status_at = EXCLUDED.status_at, \
                 signed_by = EXCLUDED.signed_by, \
                 reject_reason = EXCLUDED.reject_reason",
        )
        .bind(report.id().as_str())
        .bind(report.author().as_str())
        .bind(report.department().as_str())
        .bind(report.week().to_string())
        .bind(report.content())
        .bind(status)
        .bind(status_at)
        .bind(signed_by)
        .bind(reject_reason)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres store report failed");
            DomainError::Backend(format!("postgres store report failed: {e}"))
        })?;
        Ok(())
    }

    #[tracing::instrument(skip(self, id), fields(report_id = %id.as_str()), level = "debug")]
    async fn lookup(&self, id: &ReportId) -> Result<Option<Report>, DomainError> {
        let row = sqlx::query(
            "SELECT id, author, department, week, content, status, status_at, signed_by, reject_reason \
             FROM reports WHERE id = $1",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres lookup report failed");
            DomainError::Backend(format!("postgres lookup report failed: {e}"))
        })?;
        row.as_ref().map(row_to_report).transpose()
    }

    #[tracing::instrument(skip(self, id), fields(report_id = %id.as_str()), level = "debug")]
    async fn remove(&self, id: &ReportId) -> Result<Report, DomainError> {
        let row = sqlx::query(
            "DELETE FROM reports WHERE id = $1 \
             RETURNING id, author, department, week, content, status, status_at, signed_by, reject_reason",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres remove report failed");
            DomainError::Backend(format!("postgres remove report failed: {e}"))
        })?;
        match row {
            Some(row) => row_to_report(&row),
            None => {
                tracing::debug!("report not found");
                Err(DomainError::NotFound(format!("report {}", id.as_str())))
            }
        }
    }

    #[tracing::instrument(skip(self, author), fields(author = %author.as_str()), level = "debug")]
    async fn list_by_author(&self, author: &UserId) -> Result<Vec<Report>, DomainError> {
        let rows = sqlx::query(
            "SELECT id, author, department, week, content, status, status_at, signed_by, reject_reason \
             FROM reports WHERE author = $1",
        )
        .bind(author.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres list_by_author failed");
            DomainError::Backend(format!("postgres list_by_author failed: {e}"))
        })?;
        rows.iter().map(row_to_report).collect()
    }

    #[tracing::instrument(skip(self, department), fields(department = %department.as_str()), level = "debug")]
    async fn list_by_department(
        &self,
        department: &DepartmentId,
    ) -> Result<Vec<Report>, DomainError> {
        let rows = sqlx::query(
            "SELECT id, author, department, week, content, status, status_at, signed_by, reject_reason \
             FROM reports WHERE department = $1",
        )
        .bind(department.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres list_by_department failed");
            DomainError::Backend(format!("postgres list_by_department failed: {e}"))
        })?;
        rows.iter().map(row_to_report).collect()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn list_all(&self) -> Result<Vec<Report>, DomainError> {
        let rows = sqlx::query(
            "SELECT id, author, department, week, content, status, status_at, signed_by, reject_reason \
             FROM reports",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres list_all reports failed");
            DomainError::Backend(format!("postgres list_all reports failed: {e}"))
        })?;
        rows.iter().map(row_to_report).collect()
    }
}

impl StatusBackend for PostgresReportStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn get_status(&self) -> PortStatus {
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => PortStatus::Healthy,
            Err(e) => {
                tracing::warn!(error = %e, "postgres report-store ping failed");
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
        let store = PostgresReportStore::new(pool);

        assert!(matches!(store.get_status().await, PortStatus::Healthy));

        // Unique ids keep concurrent test runs from colliding.
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let author = format!("trainee-{suffix}");
        let signer = format!("signer-{suffix}");
        let department = format!("dept-{suffix}");
        let id = format!("report-{suffix}");

        // A signed report exercises the full status mapping: timestamp + signer.
        let week = IsoWeek::new(2026, 24).unwrap();
        let mut report = Report::new(
            ReportId::new(&id),
            UserId::new(&author),
            DepartmentId::new(&department),
            week,
            "# Week 1\n\nDid things.".to_owned(),
        );
        let at = Timestamp::now();
        report.submit(at).unwrap();
        report.sign(UserId::new(&signer), at).unwrap();
        store.store(&report).await.unwrap();

        let found = store.lookup(&ReportId::new(&id)).await.unwrap().unwrap();
        assert_eq!(*found.week(), week);
        match found.status() {
            ReviewStatus::Signed { by, at: got } => {
                assert_eq!(by.as_str(), signer);
                assert_eq!(*got, at);
            }
            other => panic!("expected Signed, got {other:?}"),
        }

        let by_author = store.list_by_author(&UserId::new(&author)).await.unwrap();
        assert_eq!(by_author.len(), 1);
        let in_dept = store
            .list_by_department(&DepartmentId::new(&department))
            .await
            .unwrap();
        assert_eq!(in_dept.len(), 1);
        assert!(in_dept.iter().all(|r| r.department().as_str() == department));
        assert!(store.list_all().await.unwrap().iter().any(|r| r.id().as_str() == id));

        let removed = store.remove(&ReportId::new(&id)).await.unwrap();
        assert_eq!(removed.id().as_str(), id);
        assert!(store.lookup(&ReportId::new(&id)).await.unwrap().is_none());
        assert!(matches!(
            store.remove(&ReportId::new(&id)).await,
            Err(DomainError::NotFound(_))
        ));
    }
}
