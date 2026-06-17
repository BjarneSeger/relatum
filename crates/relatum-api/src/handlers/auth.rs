//! Authentication handlers: login, logout, refresh.
//!
//! Each forwards to the [`Authenticator`](relatum_domain::services::auth::Authenticator)
//! held in [`AppState`](crate::state::AppState) and maps the result to a DTO or an
//! [`ApiError`](crate::error::ApiError).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;

use crate::dtos::{AuthSuccess, LoginRequest, MeView};
use crate::error::{ApiError, ErrorResponse};
use crate::extract::{BearerToken, CurrentUser};
use crate::state::AppState;

/// `POST /api/v1/login` — verify credentials and issue a session token.
#[utoipa::path(
    post,
    path = "/api/v1/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthSuccess),
        (status = 401, description = "Invalid or unrecognised SSO token", body = ErrorResponse),
    ),
)]
pub async fn login<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthSuccess>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let token = state.auth.login(req.into()).await?;
    tracing::info!("login succeeded");
    Ok(Json(token.into()))
}

/// `POST /api/v1/logout` — revoke the presented token (idempotent).
#[utoipa::path(
    post,
    path = "/api/v1/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Logged out"),
        (status = 401, description = "Missing or malformed bearer token", body = ErrorResponse),
    ),
)]
pub async fn logout<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    BearerToken(token): BearerToken,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    state.auth.logout(&token).await?;
    tracing::info!("logout");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/refresh` — rotate a valid token, returning a fresh one.
#[utoipa::path(
    post,
    path = "/api/v1/refresh",
    tag = "auth",
    responses(
        (status = 200, description = "Token rotated", body = AuthSuccess),
        (status = 401, description = "Unknown or expired session", body = ErrorResponse),
    ),
)]
pub async fn refresh<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    BearerToken(token): BearerToken,
) -> Result<Json<AuthSuccess>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let token = state.auth.refresh(&token).await?;
    tracing::info!("token refreshed");
    Ok(Json(token.into()))
}

/// `GET /api/v1/me` — the authenticated caller's own identity and role.
#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "auth",
    responses(
        (status = 200, description = "The caller's identity and role", body = MeView),
        (status = 401, description = "Missing or invalid bearer token", body = ErrorResponse),
    ),
)]
pub async fn me<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MeView>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    // Surface whether the caller has a signature on file so a UI can prompt for one
    // up front rather than waiting for a submit/sign to be rejected with 428.
    let has_signature = state.signatures.get(user.id()).await?.is_some();
    Ok(Json(MeView::from_user(&user, has_signature)))
}
