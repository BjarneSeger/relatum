//! HTTP handlers, grouped by concern.
//!
//! Each handler resolves a [`relatum_client::Client`] (anonymous, or bound to the
//! session cookie via [`WebState::authed`](crate::state::WebState::authed)), calls the
//! API, and renders an askama template or a redirect. Authorization is the API's job —
//! these handlers surface its `401`/`403`/`404` rather than re-deriving the rules.

pub mod admin;
pub mod auth;
pub mod meta;
pub mod reports;
pub mod theme;

use axum::http::HeaderMap;

/// Whether the request came from htmx (so we answer with a fragment to swap in,
/// rather than a full-page redirect). htmx sets `HX-Request: true` on every request
/// it issues; a plain `<form>` submit does not, which is what gives us the no-JS
/// fallback for free.
pub fn is_htmx(headers: &HeaderMap) -> bool {
    headers.contains_key("HX-Request")
}
