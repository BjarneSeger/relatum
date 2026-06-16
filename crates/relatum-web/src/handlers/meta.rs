//! Liveness probe and embedded static assets.
//!
//! The CSS and the vendored htmx script are baked into the binary with `include_str!`
//! and served from `/static/*`, so the frontend is a single self-contained executable
//! with no asset directory to ship alongside it.

use axum::http::header;
use axum::response::IntoResponse;

/// Liveness probe for the web process itself (does not touch the API).
pub async fn healthz() -> &'static str {
    "ok"
}

/// The application stylesheet.
pub async fn css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/app.css"),
    )
}

/// The vendored htmx runtime (htmx 2.0.9).
pub async fn htmx_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../../static/htmx-2.0.9.min.js"),
    )
}
