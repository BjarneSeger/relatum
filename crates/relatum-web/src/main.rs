//! `relatum-web` — a server-side-rendered browser frontend for the relatum API.
//!
//! Like the CLI, this binary owns no HTTP of its own: every action maps to a
//! [`relatum_client::Client`] call, so it cannot drift from the server contract. It
//! adds only the things a browser needs that the CLI does not — HTML pages (askama
//! templates), an http-only session cookie, and the SSO browser redirect dance — and
//! exposes every capability to the role that may use it (trainee, signer,
//! instructor).
//!
//! The session token lives in an **http-only** cookie (JS cannot read it); each
//! request rebuilds a client from it. Authentication is SSO-first: the web app hands
//! the API its own `/auth/callback` as the loopback `redirect_uri`, so it plays the
//! role the desktop app used to. A token-paste fallback covers dev (the `mock` SSO
//! backend) and any raw access token.

mod error;
mod handlers;
mod markdown;
mod session;
mod state;
mod view;

use std::net::SocketAddr;

use anyhow::Context;
use axum::Router;
use axum::routing::{get, post};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::state::WebState;

/// `relatum-web` command-line interface.
#[derive(Parser)]
#[command(name = "relatum-web", version, about)]
struct Args {
    /// Address the web server binds to.
    #[arg(long, env = "RELATUM_WEB_LISTEN", default_value = "0.0.0.0:8081")]
    listen: SocketAddr,

    /// Base URL of the relatum API this frontend talks to.
    #[arg(
        long,
        env = "RELATUM_WEB_API_URL",
        default_value = "http://localhost:8080"
    )]
    api_url: String,

    /// The web app's own externally-reachable base URL. Used to build the SSO
    /// `redirect_uri` (`{public_url}/auth/callback`) and to decide whether the
    /// session cookie is marked `Secure` (true when this is `https`).
    #[arg(
        long,
        env = "RELATUM_WEB_PUBLIC_URL",
        default_value = "http://localhost:8081"
    )]
    public_url: String,

    /// Departments offered in the admin assignment dropdown (comma-separated). The
    /// API remains the authority — an unknown department is rejected — so this only
    /// needs to mirror the server's configured set. Departments already in use are
    /// added automatically.
    #[arg(long, env = "RELATUM_WEB_DEPARTMENTS", value_delimiter = ',')]
    departments: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();

    // Trim, drop blanks, and dedupe while preserving first-seen order (the dropdown
    // order). `Vec::dedup` only collapses *adjacent* duplicates, so use a seen-set.
    let mut seen = std::collections::HashSet::new();
    let departments: Vec<String> = args
        .departments
        .into_iter()
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty() && seen.insert(d.clone()))
        .collect();

    let secure = args.public_url.starts_with("https://");
    let state = WebState {
        api_url: args.api_url.trim_end_matches('/').to_owned(),
        public_url: args.public_url.trim_end_matches('/').to_owned(),
        secure,
        departments,
    };

    let api = state.api_url.clone();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("binding to {}", args.listen))?;
    tracing::info!(listen = %args.listen, %api, "relatum-web listening");
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}

/// Assemble the full route table over a wired [`WebState`].
fn router(state: WebState) -> Router {
    Router::new()
        // Dashboard + auth.
        .route("/", get(handlers::reports::dashboard))
        .route(
            "/login",
            get(handlers::auth::login_page).post(handlers::auth::login_submit),
        )
        .route("/auth/sso", get(handlers::auth::sso_start))
        .route("/auth/callback", get(handlers::auth::callback))
        .route("/logout", post(handlers::auth::logout))
        // Reports.
        .route("/reports/new", get(handlers::reports::new_page))
        .route("/reports", post(handlers::reports::create))
        .route("/reports/{id}", get(handlers::reports::detail))
        .route("/reports/{id}/revise", post(handlers::reports::revise))
        .route("/reports/{id}/submit", post(handlers::reports::submit))
        .route("/reports/{id}/review", post(handlers::reports::review))
        .route("/preview", post(handlers::reports::preview))
        // Admin (instructor-only; the API enforces it).
        .route("/admin", get(handlers::admin::page))
        .route(
            "/admin/users/{id}/department",
            post(handlers::admin::assign),
        )
        .route(
            "/admin/users/{id}/department/clear",
            post(handlers::admin::clear),
        )
        // Colour-theme picker (sets the preference cookie).
        .route("/theme", post(handlers::theme::set_theme))
        // Infra + static assets.
        .route("/healthz", get(handlers::meta::healthz))
        .route("/static/app.css", get(handlers::meta::css))
        .route("/static/htmx.min.js", get(handlers::meta::htmx_js))
        .with_state(state)
}
