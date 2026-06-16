//! Database connection / pool setup.
//!
//! Holds the async PostgreSQL pool that the relational repositories in
//! [`crate::repositories`] borrow from, plus the embedded migration runner.
//! Everything here is gated on the `postgres` feature, so a build that only
//! wants the in-memory adapters need not pull in `sqlx`.

#[cfg(feature = "postgres")]
use std::time::Duration;

#[cfg(feature = "postgres")]
use relatum_domain::errors::DomainError;

/// Open a connection pool to the PostgreSQL server at `database_url`
/// (e.g. `postgres://user:pass@host:5432/dbname`).
///
/// The returned [`PgPool`](sqlx::PgPool) is cheap to clone and internally
/// multiplexed, so it is handed to each repository by value. Connection failures
/// surface as [`DomainError::Backend`].
///
/// On start-up the eager connection can fail because the Postgres host is not yet
/// resolvable or reachable — e.g. during a Kubernetes rollout where this pod
/// starts before the database Service has endpoints. The `acquire_timeout` alone
/// does not cover this: a DNS/connection error returns immediately rather than
/// being retried within the timeout. So we retry the initial connect with a
/// capped backoff for up to [`STARTUP_CONNECT_BUDGET`] before giving up.
#[cfg(feature = "postgres")]
pub async fn connect_pg(database_url: &str) -> Result<sqlx::PgPool, DomainError> {
    // The url carries the database password, so log only that we are connecting.
    tracing::info!("opening postgres connection pool");
    let deadline = std::time::Instant::now() + STARTUP_CONNECT_BUDGET;
    let mut backoff = Duration::from_millis(500);

    loop {
        let result = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_secs(30))
            .connect(database_url)
            .await;

        match result {
            Ok(pool) => return Ok(pool),
            Err(e) if std::time::Instant::now() + backoff < deadline => {
                tracing::warn!(
                    error = %e,
                    retry_in_ms = backoff.as_millis(),
                    "postgres connection failed at start-up, retrying"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
            Err(e) => {
                return Err(DomainError::Backend(format!(
                    "postgres connection failed after {}s of retries: {e}",
                    STARTUP_CONNECT_BUDGET.as_secs()
                )));
            }
        }
    }
}

/// How long [`connect_pg`] keeps retrying the initial connection before failing.
/// Sized to outlast a typical Kubernetes Service becoming ready.
#[cfg(feature = "postgres")]
const STARTUP_CONNECT_BUDGET: Duration = Duration::from_secs(90);

/// Apply every pending migration in `crates/relatum-infra/migrations` to `pool`.
///
/// The migration set is embedded at compile time, so no SQL files need to ship
/// alongside the binary. Run this once at start-up before serving requests.
#[cfg(feature = "postgres")]
pub async fn run_migrations(pool: &sqlx::PgPool) -> Result<(), DomainError> {
    tracing::info!("applying postgres migrations");
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "postgres migration failed");
            DomainError::Backend(format!("migration failed: {e}"))
        })?;
    tracing::info!("postgres migrations applied");
    Ok(())
}
