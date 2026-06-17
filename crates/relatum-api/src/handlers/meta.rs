//! Service-metadata and health handlers: info, healthz, readyz.
//!
//! `info` and `healthz` touch only metadata; `readyz` probes the backing stores,
//! so it additionally bounds the storage ports by
//! [`StatusBackend`](relatum_domain::ports::status::StatusBackend).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::status::StatusBackend;
use relatum_domain::ports::userstorage::UserStorage;

use crate::dtos::ApiInfo;
use crate::error::{ApiError, ErrorResponse};
use crate::state::AppState;

/// `GET /api/v1/info` — name and version of the running service.
#[utoipa::path(
    get,
    path = "/api/v1/info",
    tag = "meta",
    responses(
        (status = 200, description = "Service metadata", body = ApiInfo),
        (status = 500, description = "Internal error", body = ErrorResponse),
    ),
)]
pub async fn info<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
) -> Result<Json<ApiInfo>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let info = state.meta.info().await?;
    Ok(Json(info.into()))
}

/// `GET /api/v1/healthz` — liveness probe (`200` once started).
#[utoipa::path(
    get,
    path = "/api/v1/healthz",
    tag = "meta",
    responses(
        (status = 200, description = "Service is live"),
        (status = 502, description = "Service is not live"),
    ),
)]
pub async fn healthz<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    state.meta.health().await?;
    Ok(StatusCode::OK)
}

/// `GET /api/v1/readyz` — readiness probe (`200` when it can serve traffic).
#[utoipa::path(
    get,
    path = "/api/v1/readyz",
    tag = "meta",
    responses(
        (status = 200, description = "Service is ready"),
        (status = 502, description = "Service is up but not ready", body = ErrorResponse),
    ),
)]
pub async fn readyz<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + StatusBackend + Clone + 'static,
    S: SessionRepository + StatusBackend + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    state.meta.readiness().await?;
    Ok(StatusCode::OK)
}
