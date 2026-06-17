//! `relatum-server` — the HTTP backend binary.
//!
//! This is the composition root: it reads a config file, picks the storage
//! backends, wires the matching [`relatum-infra`](relatum_infra) adapters into the
//! domain services, and serves the [`relatum-api`](relatum_api) router.
//!
//! The whole stack is generic over the five outbound ports and the port traits use
//! `impl Future` (not object-safe), so backend choice is resolved by *matching on
//! config and monomorphizing* a generic [`serve`] — not via `dyn` indirection. The
//! varying axes are the data store (Postgres / in-memory), the session store
//! (Valkey / in-memory) and the SSO provider (OIDC / disabled); the id generator is
//! fixed. Authentication is SSO-only, so there is no password hasher.
//!
//! Users are provisioned from the LDAP directory by a periodic sync rather than
//! registered out of band; their department is assigned over the API. The set of
//! valid departments is hard-coded at startup from config.

mod config;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use relatum_api::state::AppState;
use relatum_domain::models::department::DepartmentRegistry;
use relatum_domain::models::ids::DepartmentId;
use relatum_domain::ports::ephemeral::EphemeralStore;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::status::StatusBackend;
use relatum_domain::ports::userstorage::UserStorage;
use relatum_domain::services::admin::UserAdmin;
use relatum_domain::services::auth::Authenticator;
use relatum_domain::services::meta::MetaService;
use relatum_domain::services::report::ReportService;
use relatum_domain::services::signature::SignatureService;
use relatum_domain::services::sync::DirectorySync;
use relatum_infra::db;
use relatum_infra::directory::{LdapConfig, LdapDirectory};
use relatum_infra::ids::UuidIdGenerator;
use relatum_infra::repositories::ephemeral::{InMemoryTtlEphemeralStore, ValkeyEphemeralStore};
use relatum_infra::repositories::report::{InMemoryReports, PostgresReportStore};
use relatum_infra::repositories::session::{InMemoryTtlSessionStore, ValkeySessionStore};
use relatum_infra::repositories::signature::{InMemorySignatures, PostgresSignatureStore};
use relatum_infra::repositories::user::{InMemoryUsers, PostgresUserStore};
use relatum_infra::sso::{DisabledSso, OidcFlow, OidcSso};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::config::{DataStore, DirectoryStore, SessionStore, SsoStore};

#[cfg(feature = "dev")]
use relatum_domain::models::users::User;
#[cfg(feature = "dev")]
use relatum_domain::ports::sso_connector::SsoIdentity;
#[cfg(feature = "dev")]
use relatum_domain::testing::{MockSSO, dev_token, dev_user_catalogue};

/// `relatum-server` command-line interface.
///
/// With no subcommand the binary loads its configuration and serves; the
/// subcommands are one-shot tools that exit when done.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Path to the config file. Falls back to `$RELATUM_CONFIG`, then
    /// `config.toml` in the working directory (ignored if absent).
    #[arg(short, long, env = "RELATUM_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Write the documented config template to a file, or stdout when no path
    /// is given.
    GenerateConfig { path: Option<PathBuf> },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        // Show the target (module) so api / infra / library lines are distinguishable
        // for `RUST_LOG` filtering.
        .with_target(true)
        // Emit a line when a span closes. The API's per-request span (see
        // `relatum_api::routes`) records its status+latency before closing, so this is
        // what turns each request into a single log line — at the span's own level, so
        // health-probe spans (debug) stay silent at the info baseline.
        .with_span_events(FmtSpan::CLOSE)
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "starting relatum-server"
    );

    let cli = Cli::parse();

    // One-shot subcommands exit here; falling through means "serve".
    match cli.command {
        Some(Command::GenerateConfig { path }) => return generate_config(path),
        None => {}
    }

    let config = config::ServerConfig::load(cli.config.as_deref())?;
    tracing::info!("configuration loaded");

    // The fixed set of departments users may be assigned to.
    let departments =
        DepartmentRegistry::new(config.departments.iter().cloned().map(DepartmentId::new));

    // Fixed adapter — there is one production choice.
    let ids = UuidIdGenerator;

    // The external directory the periodic user sync provisions from. Owned here and
    // moved into whichever data arm runs.
    let directory = config.directory;

    // First axis: the data store backing users + reports. Each arm fixes the
    // concrete `U`/`R` types, spawns the directory sync against that user store, then
    // defers the session-store and SSO choices.
    match config.data {
        DataStore::Postgres { url } => {
            let pool = db::connect_pg(&url).await?;
            db::run_migrations(&pool).await?;
            let users = PostgresUserStore::new(pool.clone());
            let signatures = PostgresSignatureStore::new(pool.clone());
            let reports = PostgresReportStore::new(pool);
            spawn_directory_sync(directory, users.clone()).await;
            serve_with_sessions(
                config.listen,
                config.sessions,
                config.sso,
                departments,
                users,
                reports,
                signatures,
                ids,
            )
            .await
        }
        DataStore::Memory => {
            let users = InMemoryUsers::new();
            let reports = InMemoryReports::new();
            let signatures = InMemorySignatures::new();
            spawn_directory_sync(directory, users.clone()).await;
            serve_with_sessions(
                config.listen,
                config.sessions,
                config.sso,
                departments,
                users,
                reports,
                signatures,
                ids,
            )
            .await
        }
    }
}

/// Spawn the background task that reconciles the external directory into the user
/// store on an interval. Provisioning is SSO-login's prerequisite — a login is only
/// accepted for a user the sync has already created — so this is what makes logins
/// possible at all.
///
/// With the directory disabled this only logs that no sync runs (users must be
/// present by other means); otherwise it builds the [`LdapDirectory`] adapter, wraps
/// it in a [`DirectorySync`], and ticks it. The very first tick fires immediately, so
/// an initial reconciliation happens at startup; a failed pass is logged and retried
/// on the next tick rather than aborting the loop.
async fn spawn_directory_sync<U>(directory: DirectoryStore, users: U)
where
    U: UserStorage + Clone + 'static,
{
    match directory {
        DirectoryStore::Disabled => {
            tracing::info!(
                "no directory backend configured; users must be provisioned by other means"
            );
        }
        #[cfg(feature = "dev")]
        DirectoryStore::Mock => {
            tracing::warn!(
                "DEV MODE: directory.backend=mock — seeding fixed mock users into the store; never use in production"
            );
            for (id, marker, department) in dev_user_catalogue() {
                let username = id.as_str().to_owned();
                if let Err(e) = users
                    .store(User::new(id, username, marker, department))
                    .await
                {
                    tracing::error!(error = %e, "failed to seed dev user");
                }
            }
        }
        DirectoryStore::Ldap(settings) => {
            let interval_secs = settings.interval_secs;
            let ldap = LdapDirectory::new(LdapConfig {
                url: settings.url,
                bind_dn: settings.bind_dn,
                bind_password: settings.bind_password,
                user_base: settings.user_base,
                user_filter: settings.user_filter,
                id_attr: settings.id_attr,
                username_attr: settings.username_attr,
                group_attr: settings.group_attr,
                instructor_group: settings.instructor_group,
                trainee_group: settings.trainee_group,
            });
            let sync = DirectorySync::new(ldap, users);
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
                loop {
                    ticker.tick().await;
                    match sync.sync().await {
                        Ok(summary) => tracing::info!(
                            added = summary.added,
                            updated = summary.updated,
                            removed = summary.removed,
                            "directory sync complete"
                        ),
                        Err(e) => {
                            tracing::warn!(error = %e, "directory sync failed; will retry next tick")
                        }
                    }
                }
            });
        }
    }
}

/// Second axis: pick the session store, then defer the SSO choice. Generic over
/// the data-store types resolved by the caller.
// Flat wiring dispatch: the stores and config join as separate args rather than a
// bundling struct, which would only obscure the per-backend monomorphization.
#[allow(clippy::too_many_arguments)]
async fn serve_with_sessions<U, R, G>(
    listen: SocketAddr,
    sessions: SessionStore,
    sso: SsoStore,
    departments: DepartmentRegistry,
    users: U,
    reports: R,
    signatures: G,
    ids: UuidIdGenerator,
) -> anyhow::Result<()>
where
    U: UserStorage + StatusBackend + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    let ttl = sessions.ttl();
    match sessions {
        SessionStore::Redis { url, .. } => {
            let store = ValkeySessionStore::connect(&url, ttl).await?;
            // The SSO browser-flow state shares the session store's redis (same URL),
            // so a login started on one replica can complete on any other.
            let ephemeral = ValkeyEphemeralStore::connect(&url).await?;
            serve_with_sso(
                listen,
                users,
                store,
                ephemeral,
                ids,
                sso,
                departments,
                reports,
                signatures,
            )
            .await
        }
        SessionStore::Memory { .. } => {
            let store = InMemoryTtlSessionStore::new(ttl);
            // Reclaim memory for tokens that are stored and never looked up again;
            // lazy eviction on lookup already keeps results correct.
            store.spawn_sweeper(Duration::from_secs(60));
            // The in-process SSO flow state is single-node only, matching memory
            // sessions; its sweeper reclaims abandoned (never-completed) logins.
            let ephemeral = InMemoryTtlEphemeralStore::new();
            ephemeral.spawn_sweeper(Duration::from_secs(60));
            serve_with_sso(
                listen,
                users,
                store,
                ephemeral,
                ids,
                sso,
                departments,
                reports,
                signatures,
            )
            .await
        }
    }
}

/// Third axis: pick the SSO provider, then hand the fully-typed adapter set to
/// [`serve`]. Generic over the data-store and session-store types resolved by the
/// callers.
// One wiring arg past clippy's threshold: the ephemeral store joins the other ports
// that the SSO arm monomorphizes over. Bundling them would obscure the flat dispatch.
#[allow(clippy::too_many_arguments)]
async fn serve_with_sso<U, S, E, R, G>(
    listen: SocketAddr,
    users: U,
    sessions: S,
    ephemeral: E,
    ids: UuidIdGenerator,
    sso: SsoStore,
    departments: DepartmentRegistry,
    reports: R,
    signatures: G,
) -> anyhow::Result<()>
where
    U: UserStorage + StatusBackend + Clone + 'static,
    S: SessionRepository + StatusBackend + Clone + 'static,
    E: EphemeralStore + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    match sso {
        SsoStore::Disabled => {
            serve(
                listen,
                users,
                sessions,
                ids,
                DisabledSso,
                departments,
                reports,
                signatures,
            )
            .await
        }
        SsoStore::Oidc {
            userinfo_url,
            authorize_url,
            token_url,
            client_id,
            client_secret,
            scopes,
            public_url,
            allowed_redirects,
        } => {
            // An empty allowlist accepts only loopback redirects (the native CLI), so
            // the browser/web SSO flow is effectively disabled. Surface this at startup —
            // it is the secure default, but also the upgrade trap for deployments that
            // previously relied on browser SSO without setting an origin.
            if allowed_redirects.is_empty() {
                tracing::warn!(
                    "sso.allowed_redirects is empty — only loopback (native CLI) redirects are accepted; \
                     set RELATUM_SSO_ALLOWED_REDIRECTS to the web frontend's origin to enable browser SSO"
                );
            }
            // The OIDC provider keeps its short-lived flow state in `ephemeral`, which
            // follows the session backend (shared redis for HA, in-process otherwise).
            let provider = OidcSso::new(
                userinfo_url,
                OidcFlow {
                    authorize_url,
                    token_url,
                    client_id,
                    client_secret,
                    scopes,
                    public_url,
                    allowed_redirects,
                },
                ephemeral,
            )?;
            serve(
                listen,
                users,
                sessions,
                ids,
                provider,
                departments,
                reports,
                signatures,
            )
            .await
        }
        #[cfg(feature = "dev")]
        SsoStore::Mock => {
            tracing::warn!(
                "DEV MODE: sso.backend=mock — accepting fixed dev tokens (tok-<id>); never use in production"
            );
            let provider = MockSSO::default();
            for (id, _marker, _department) in dev_user_catalogue() {
                let token = dev_token(id.as_str());
                provider.register(&token, SsoIdentity { id });
            }
            serve(
                listen,
                users,
                sessions,
                ids,
                provider,
                departments,
                reports,
                signatures,
            )
            .await
        }
    }
}

/// Assemble the domain services into [`AppState`], build the router and serve it.
///
/// Generic over all six ports; the caller's `match` arms monomorphize one instance
/// per backend combination.
// Flat wiring dispatch: every port arrives as its own arg, mirroring the other
// `serve_*` stages; bundling them would not make the composition root clearer.
#[allow(clippy::too_many_arguments)]
async fn serve<U, S, I, P, R, G>(
    listen: SocketAddr,
    users: U,
    sessions: S,
    ids: I,
    sso: P,
    departments: DepartmentRegistry,
    reports: R,
    signatures: G,
) -> anyhow::Result<()>
where
    U: UserStorage + StatusBackend + Clone + 'static,
    S: SessionRepository + StatusBackend + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    // The periodic LDAP directory sync that provisions users is spawned by `main`
    // (see `spawn_directory_sync`) before this runs, against the same user store.

    // Build the meta service first: it probes clones of the stores that the other
    // services take ownership of below.
    let meta = MetaService::new(
        "relatum",
        env!("CARGO_PKG_VERSION"),
        users.clone(),
        sessions.clone(),
        reports.clone(),
    );

    let signature_service = SignatureService::new(signatures.clone());
    let state = AppState::new(
        Authenticator::new(users.clone(), sessions, ids.clone(), sso),
        meta,
        ReportService::new(reports, users.clone(), ids, signatures),
        UserAdmin::new(users, departments),
        signature_service,
    );

    let app = relatum_api::router(state);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding to {listen}"))?;
    tracing::info!(%listen, "relatum-server listening");
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}

/// Write the documented config template to `path`, or print it to stdout when no
/// path is given. Refuses to overwrite an existing file.
fn generate_config(path: Option<PathBuf>) -> anyhow::Result<()> {
    let template = config::template();
    match path {
        None => {
            print!("{template}");
            Ok(())
        }
        Some(path) => {
            if path.exists() {
                anyhow::bail!("refusing to overwrite existing file {}", path.display());
            }
            std::fs::write(&path, template)
                .with_context(|| format!("writing config template to {}", path.display()))?;
            eprintln!("wrote config template to {}", path.display());
            Ok(())
        }
    }
}
