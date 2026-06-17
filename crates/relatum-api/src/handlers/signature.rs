//! Self-service signature handlers: set (or replace) and get the caller's own.
//!
//! A signature is what a future PDF export stamps onto a report, so only the roles
//! that appear on one — a trainee (as author) or a signer (as approver) — may
//! register one; read-only instructors and inert users are refused. The target is
//! always the authenticated caller ([`CurrentUser`]), never a path/body id, so this
//! cannot be turned into signature account-takeover. There is no delete: a signature
//! is set or replaced, never removed, so the submit/sign gate's guarantee holds.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use relatum_domain::models::signature::Signature;
use relatum_domain::models::users::{Role, User};
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;

use crate::dtos::{SetSignatureRequest, SignatureView};
use crate::error::{ApiError, ErrorResponse};
use crate::extract::CurrentUser;
use crate::state::AppState;

/// Only the roles that appear on a report may register a signature: a trainee (as
/// author) or a signer (as approver). Instructors are read-only and an inert user
/// has no role.
fn require_author_or_signer(user: &User) -> Result<(), ApiError> {
    match user.role() {
        Some(Role::Trainee { .. }) | Some(Role::Signer { .. }) => Ok(()),
        Some(Role::Instructor { .. }) => Err(ApiError::Forbidden(
            "instructors do not author or sign reports, so cannot register a signature".into(),
        )),
        None => Err(ApiError::Forbidden(
            "assign a department before registering a signature".into(),
        )),
    }
}

/// `PUT /api/v1/me/signature` — set or replace the caller's signature.
#[utoipa::path(
    put,
    path = "/api/v1/me/signature",
    tag = "signatures",
    operation_id = "set_signature",
    request_body = SetSignatureRequest,
    responses(
        (status = 204, description = "Signature stored"),
        (status = 400, description = "Malformed base64 or not a valid image", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Instructors and inert users may not register a signature", body = ErrorResponse),
    ),
)]
pub async fn set<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SetSignatureRequest>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    require_author_or_signer(&user)?;
    let bytes = req.decode().map_err(|e| {
        ApiError::BadRequest(format!("signature data_base64 is not valid base64: {e}"))
    })?;
    // `Signature::new` validates the magic number and size cap; an invalid image is a
    // domain `Invalid` -> 400.
    let signature = Signature::new(req.format.into(), bytes)?;
    state.signatures.set(user.id(), signature).await?;
    tracing::info!("signature set");
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/me/signature` — the caller's signature, or 404 if none on file.
#[utoipa::path(
    get,
    path = "/api/v1/me/signature",
    tag = "signatures",
    operation_id = "get_signature",
    responses(
        (status = 200, description = "The caller's signature", body = SignatureView),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "No signature on file", body = ErrorResponse),
    ),
)]
pub async fn get<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<SignatureView>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    match state.signatures.get(user.id()).await? {
        Some(stored) => Ok(Json(SignatureView::from(&stored))),
        None => Err(ApiError::NotFound("no signature on file".into())),
    }
}
