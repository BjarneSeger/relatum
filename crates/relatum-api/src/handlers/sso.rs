//! SSO browser-login handlers: advertise, start, complete, and exchange.
//!
//! The server drives the whole OAuth authorization-code dance with a confidential
//! client, so a browser frontend only opens a URL and catches the result on its own
//! callback. Endpoints:
//!
//! - [`info`] (`GET /api/v1/auth/sso`) — JSON metadata the client polls to decide
//!   whether to show the SSO button. Documented in the OpenAPI spec, so the typed
//!   client gets it.
//! - [`start`] (`GET /api/v1/auth/sso/start`) — the browser lands here with the
//!   app's `redirect_uri` + `state`; it is redirected on to the IdP.
//! - [`callback`] (`GET /api/v1/auth/sso/callback`) — the IdP redirects back here
//!   with a `code`; the server completes the login and redirects the browser to the
//!   app carrying a **single-use handoff code** (never the session token itself).
//! - [`exchange`] (`POST /api/v1/auth/sso/exchange`) — the app redeems that handoff
//!   code back-channel for the session token. JSON, so it is documented and the typed
//!   client gets it.
//!
//! `start`/`callback` return HTTP redirects rather than JSON, so they are mounted as
//! plain routes (see [`crate::routes::router`]) and kept out of the OpenAPI document.

use axum::Json;
use axum::extract::{Query, State};
use axum::response::Redirect;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;
use serde::Deserialize;

use crate::dtos::{AuthSuccess, SsoExchangeRequest, SsoInfo};
use crate::error::{ApiError, ErrorResponse};
use crate::state::AppState;

/// `GET /api/v1/auth/sso` — whether SSO is available and where login starts.
#[utoipa::path(
    get,
    path = "/api/v1/auth/sso",
    tag = "auth",
    operation_id = "sso_info",
    responses(
        (status = 200, description = "SSO availability and start URL", body = SsoInfo),
    ),
)]
pub async fn info<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
) -> Result<Json<SsoInfo>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    Ok(Json(state.auth.sso_metadata().into()))
}

/// Query for [`start`]: the app's loopback URL and its CSRF nonce.
#[derive(Debug, Deserialize)]
pub struct StartQuery {
    /// The client's loopback URL to return the browser to once done.
    redirect_uri: String,
    /// The app's own nonce, echoed back so the app can verify the response.
    state: String,
}

/// `GET /api/v1/auth/sso/start` — redirect the browser on to the IdP login.
pub async fn start<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
    Query(query): Query<StartQuery>,
) -> Result<Redirect, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    // Carry the app's nonce on its loopback URL so it travels through the IdP leg and
    // comes back for the app to verify.
    let app_redirect = format!("{}?state={}", query.redirect_uri, query.state);
    let authorize_url = state.auth.sso_begin(&app_redirect).await?;
    Ok(Redirect::to(&authorize_url))
}

/// Query for [`callback`]: the IdP's authorization code and matching state.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    /// The authorization code to exchange for an access token.
    code: String,
    /// The state the server handed the IdP, matching a pending login.
    state: String,
}

/// `GET /api/v1/auth/sso/callback` — complete the login and return to the app with a
/// single-use handoff code (not the session token).
pub async fn callback<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
    Query(query): Query<CallbackQuery>,
) -> Result<Redirect, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    let (handoff, app_redirect) = state.auth.sso_complete(&query.code, &query.state).await?;
    // `app_redirect` already carries `?state=<nonce>`; append the single-use handoff
    // code. The session token never travels in the URL — the app swaps the code for it
    // back-channel via `exchange` below.
    let target = format!("{app_redirect}&code={handoff}");
    Ok(Redirect::to(&target))
}

/// `POST /api/v1/auth/sso/exchange` — redeem a single-use handoff code for a session.
#[utoipa::path(
    post,
    path = "/api/v1/auth/sso/exchange",
    tag = "auth",
    operation_id = "sso_exchange",
    request_body = SsoExchangeRequest,
    responses(
        (status = 200, description = "Handoff redeemed; session issued", body = AuthSuccess),
        (status = 401, description = "Unknown, used, or expired handoff code", body = ErrorResponse),
    ),
)]
pub async fn exchange<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
    Json(req): Json<SsoExchangeRequest>,
) -> Result<Json<AuthSuccess>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    let token = state.auth.sso_exchange(&req.code).await?;
    tracing::info!("sso handoff redeemed");
    Ok(Json(token.into()))
}
