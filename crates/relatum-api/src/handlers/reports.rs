//! Report-workflow handlers: create, get, list, revise, submit, review.
//!
//! Every handler resolves the acting user via [`CurrentUser`] and forwards to the
//! [`ReportService`](relatum_domain::services::report::ReportService); the domain
//! enforces author/signer authorization, surfacing `Forbidden`/`NotFound` which
//! map straight to `403`/`404`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use relatum_domain::models::ids::ReportId;
use relatum_domain::models::report::ReviewDecision;
use relatum_domain::models::week::IsoWeek;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;

use crate::dtos::{
    CreateReportRequest, CreatedReport, ReportView, ReviewRequest, ReviseReportRequest,
};
use crate::error::{ApiError, ErrorResponse};
use crate::extract::CurrentUser;
use crate::state::AppState;

/// `POST /api/v1/reports` — start a draft authored by the current user.
#[utoipa::path(
    post,
    path = "/api/v1/reports",
    tag = "reports",
    request_body = CreateReportRequest,
    responses(
        (status = 201, description = "Draft created", body = CreatedReport),
        (status = 400, description = "Malformed ISO week", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Only a trainee may author reports", body = ErrorResponse),
        (status = 404, description = "Author not found", body = ErrorResponse),
        (status = 409, description = "A report already exists for this week", body = ErrorResponse),
    ),
)]
pub async fn create<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateReportRequest>,
) -> Result<(StatusCode, Json<CreatedReport>), ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    // A malformed week is a domain `Invalid` -> 400 before we touch storage.
    let week: IsoWeek = req.week.parse()?;
    let id = state
        .reports
        .create_draft(user.id(), week, req.content)
        .await?;
    tracing::info!(
        report_id = %id.as_str(),
        week = %req.week,
        "report created"
    );
    Ok((StatusCode::CREATED, Json(id.into())))
}

/// `GET /api/v1/reports` — the current user's reports (authored, their department
/// queue, or all of them for an instructor).
#[utoipa::path(
    get,
    path = "/api/v1/reports",
    tag = "reports",
    responses(
        (status = 200, description = "The caller's reports", body = [ReportView]),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
    ),
)]
pub async fn list<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<ReportView>>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let reports = state.reports.list_for(user.id()).await?;
    Ok(Json(reports.into_iter().map(ReportView::from).collect()))
}

/// `GET /api/v1/reports/{id}` — a single report visible to the current user.
#[utoipa::path(
    get,
    path = "/api/v1/reports/{id}",
    tag = "reports",
    params(("id" = String, Path, description = "Report id")),
    responses(
        (status = 200, description = "The report", body = ReportView),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Not permitted to view this report", body = ErrorResponse),
        (status = 404, description = "Report not found", body = ErrorResponse),
    ),
)]
pub async fn get<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ReportView>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let report = state.reports.get(user.id(), &ReportId::new(id)).await?;
    Ok(Json(report.into()))
}

/// `PUT /api/v1/reports/{id}` — replace a draft/rejected report's markdown.
#[utoipa::path(
    put,
    path = "/api/v1/reports/{id}",
    tag = "reports",
    params(("id" = String, Path, description = "Report id")),
    request_body = ReviseReportRequest,
    responses(
        (status = 204, description = "Revised"),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Not the author", body = ErrorResponse),
        (status = 404, description = "Report not found", body = ErrorResponse),
    ),
)]
pub async fn revise<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<ReviseReportRequest>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let id = ReportId::new(id);
    state.reports.revise(user.id(), &id, req.content).await?;
    tracing::info!(report_id = %id.as_str(), "report revised");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/reports/{id}/submit` — submit the report into its department queue.
#[utoipa::path(
    post,
    path = "/api/v1/reports/{id}/submit",
    tag = "reports",
    params(("id" = String, Path, description = "Report id")),
    responses(
        (status = 204, description = "Submitted"),
        (status = 400, description = "Report is empty or in a non-submittable state", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Not the author", body = ErrorResponse),
        (status = 404, description = "Report not found", body = ErrorResponse),
        (status = 428, description = "Author has no signature on file — register one first", body = ErrorResponse),
    ),
)]
pub async fn submit<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let id = ReportId::new(id);
    state.reports.submit(user.id(), &id).await?;
    tracing::info!(report_id = %id.as_str(), "report submitted");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/reports/{id}/review` — sign or reject (signers in the report's
/// department only).
#[utoipa::path(
    post,
    path = "/api/v1/reports/{id}/review",
    tag = "reports",
    params(("id" = String, Path, description = "Report id")),
    request_body = ReviewRequest,
    responses(
        (status = 204, description = "Decision applied"),
        (status = 400, description = "Report is not awaiting a signature", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Not a signer in this report's department", body = ErrorResponse),
        (status = 404, description = "Report not found", body = ErrorResponse),
        (status = 428, description = "Signer has no signature on file — register one first", body = ErrorResponse),
    ),
)]
pub async fn review<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<ReviewRequest>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let id = ReportId::new(id);
    // Record which decision was applied, but never the free-form rejection `reason`.
    match req.decision.into() {
        ReviewDecision::Sign => {
            state.reports.sign(user.id(), &id).await?;
        }
        ReviewDecision::Reject { reason } => {
            state.reports.reject(user.id(), &id, reason).await?;
        }
    };
    tracing::info!(
        report_id = %id.as_str(),
        "report reviewed"
    );
    Ok(StatusCode::NO_CONTENT)
}
