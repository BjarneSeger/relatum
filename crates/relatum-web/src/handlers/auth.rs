//! Authentication: the login page, the SSO browser dance, and logout.
//!
//! Authentication is SSO-first. The web app plays the role the old desktop app did:
//! it hands the API its own `/auth/callback` as the loopback `redirect_uri`, mints a
//! `state` nonce to bind the round trip, and catches the single-use handoff code the
//! API redirects back with, swapping it back-channel for the session token (the token
//! never travels in the URL). A token-paste form is kept as a fallback so the `mock` SSO
//! dev backend (and any raw access token) still works.

use axum::extract::{Form, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use relatum_client::ClientError;
use serde::Deserialize;
use uuid::Uuid;

use crate::error::WebError;
use crate::session::{self, SSO_STATE_COOKIE};
use crate::state::WebState;
use crate::view::{Login, Theme};
use askama::Template;

/// `GET /login` — show the SSO button (when available) and the token fallback.
pub async fn login_page(
    State(state): State<WebState>,
    jar: CookieJar,
) -> Result<Response, WebError> {
    if session::token(&jar).is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    Ok(Html(login_html(&state, Theme::from_cookie(&jar), None).await?).into_response())
}

#[derive(Deserialize)]
pub struct LoginForm {
    token: String,
}

/// `POST /login` — exchange a pasted access token for a session and set the cookie.
pub async fn login_submit(
    State(state): State<WebState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Result<Response, WebError> {
    let mut client = state.anon_client();
    match client.login(form.token.trim()).await {
        Ok(()) => {
            let token = client
                .token()
                .expect("a token is set after a successful login")
                .to_owned();
            let jar = jar.add(session::session_cookie(token, state.secure));
            Ok((jar, Redirect::to("/")).into_response())
        }
        // A bad token is a form error, not a server error: re-render login with a note.
        Err(ClientError::Api { status: 401, .. }) => {
            let theme = Theme::from_cookie(&jar);
            let html = login_html(
                &state,
                theme,
                Some("That token was not accepted.".to_owned()),
            )
            .await?;
            Ok((axum::http::StatusCode::UNAUTHORIZED, Html(html)).into_response())
        }
        Err(err) => Err(err.into()),
    }
}

/// `GET /auth/sso` — begin the browser SSO flow.
///
/// Mints a nonce, stashes it in a short-lived cookie, and redirects to the API's
/// `start` endpoint with our callback as the `redirect_uri`. The API drives the IdP
/// and redirects the browser back to `/auth/callback` carrying `state` + a single-use
/// handoff `code`.
pub async fn sso_start(
    State(state): State<WebState>,
    jar: CookieJar,
) -> Result<Response, WebError> {
    let nonce = Uuid::new_v4().to_string();
    let redirect_uri = format!("{}/auth/callback", state.public_url);
    let start_url = format!(
        "{}/api/v1/auth/sso/start?redirect_uri={}&state={}",
        state.api_url,
        percent_encode(&redirect_uri),
        percent_encode(&nonce),
    );
    let jar = jar.add(session::sso_state_cookie(nonce, state.secure));
    Ok((jar, Redirect::to(&start_url)).into_response())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

/// `GET /auth/callback` — finish SSO: verify the nonce, redeem the handoff code for a
/// session back-channel, store it, go home.
///
/// The API redirects here with a **single-use, short-lived handoff `code`** — never
/// the session token (see `relatum-api/src/handlers/sso.rs`). Only that ephemeral code
/// touches the browser/URL; we swap it for the real session via a server-to-server
/// [`Client::exchange_sso`] call and put the session in an HttpOnly cookie. So even a
/// reverse proxy that logs query strings only ever sees a code that is already spent
/// (and TTL-bound) by the time it could be misused.
pub async fn callback(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, WebError> {
    // The nonce is single-use: clear it whatever the outcome.
    let expected = jar.get(SSO_STATE_COOKIE).map(|c| c.value().to_owned());
    let jar = jar.remove(session::removal_cookie(SSO_STATE_COOKIE));

    if let Some(err) = query.error {
        return login_fail(&state, jar, format!("SSO sign-in failed: {err}")).await;
    }
    match (query.state.as_deref(), expected.as_deref()) {
        (Some(got), Some(want)) if got == want => {}
        _ => {
            return login_fail(
                &state,
                jar,
                "SSO sign-in could not be verified; please retry.".to_owned(),
            )
            .await;
        }
    }
    let Some(code) = query.code.filter(|c| !c.is_empty()) else {
        return login_fail(&state, jar, "SSO did not return a session.".to_owned()).await;
    };

    // Redeem the handoff code for the session token, back-channel.
    let mut client = state.anon_client();
    match client.exchange_sso(&code).await {
        Ok(()) => {
            let token = client
                .token()
                .expect("a token is set after a successful exchange")
                .to_owned();
            let jar = jar.add(session::session_cookie(token, state.secure));
            Ok((jar, Redirect::to("/")).into_response())
        }
        Err(_) => {
            login_fail(
                &state,
                jar,
                "SSO sign-in could not be completed; please retry.".to_owned(),
            )
            .await
        }
    }
}

/// Re-render the login page with an error (carrying along the cleared `sso_state`
/// cookie in `jar`).
async fn login_fail(
    state: &WebState,
    jar: CookieJar,
    message: String,
) -> Result<Response, WebError> {
    let html = login_html(state, Theme::from_cookie(&jar), Some(message)).await?;
    Ok((axum::http::StatusCode::BAD_REQUEST, jar, Html(html)).into_response())
}

/// `POST /logout` — revoke the session server-side (best effort) and clear the cookie.
pub async fn logout(State(state): State<WebState>, jar: CookieJar) -> Result<Response, WebError> {
    if let Ok(mut client) = state.authed(&jar) {
        let _ = client.logout().await;
    }
    let jar = jar.remove(session::removal_cookie(session::SESSION_COOKIE));
    Ok((jar, Redirect::to("/login")).into_response())
}

/// Render the login page, probing the API for SSO availability (defaulting to the
/// token form if that probe fails).
async fn login_html(
    state: &WebState,
    theme: Theme,
    error: Option<String>,
) -> Result<String, WebError> {
    let sso_enabled = state
        .anon_client()
        .sso_info()
        .await
        .map(|info| info.enabled)
        .unwrap_or(false);
    Ok(Login {
        theme,
        sso_enabled,
        error,
    }
    .render()?)
}

/// Percent-encode a query-string value (RFC 3986 unreserved set passes through).
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
