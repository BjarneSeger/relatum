//! The colour-theme picker's form target.
//!
//! The picker in the topbar is a native `<select>`. With JS, htmx posts the choice
//! here on `change` and we answer with `HX-Refresh: true` so the page reloads and the
//! server re-renders `<html data-theme="…">` from the new cookie. Without JS, a
//! `<noscript>` submit button posts the same form and we redirect back. Either way the
//! pick is a non-secret display preference kept in a persistent cookie.

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use serde::Deserialize;

use crate::handlers::is_htmx;
use crate::session;
use crate::state::WebState;
use crate::view::Theme;

#[derive(Deserialize)]
pub struct ThemeForm {
    theme: String,
}

/// `POST /theme` — persist the chosen theme in a cookie, then re-render with it.
pub async fn set_theme(
    State(state): State<WebState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(form): Form<ThemeForm>,
) -> Response {
    // Normalise through `Theme` so only a known value is ever stored.
    let theme = Theme::parse(&form.theme);
    let jar = jar.add(session::theme_cookie(theme.attr().to_owned(), state.secure));

    if is_htmx(&headers) {
        // htmx does a full-page reload on this header, so the server re-renders the
        // whole document (incl. `<html data-theme>`) under the new cookie.
        (jar, [("HX-Refresh", "true")], StatusCode::OK).into_response()
    } else {
        // No-JS fallback: bounce back to where the form was submitted from.
        (jar, Redirect::to(&back_to(&headers, &state))).into_response()
    }
}

/// Where to send the browser after a no-JS theme change: back to the referring page
/// when it is one of ours, else home. Guards against an open redirect by only honouring
/// a `Referer` under our own public URL.
fn back_to(headers: &HeaderMap, state: &WebState) -> String {
    headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .filter(|r| *r == state.public_url || r.starts_with(&format!("{}/", state.public_url)))
        .map(|r| r.to_owned())
        .unwrap_or_else(|| "/".to_owned())
}
