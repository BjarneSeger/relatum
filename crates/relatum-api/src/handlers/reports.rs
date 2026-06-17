//! Report-workflow handlers: create, get, list, revise, submit, review.
//!
//! Every handler resolves the acting user via [`CurrentUser`] and forwards to the
//! [`ReportService`](relatum_domain::services::report::ReportService); the domain
//! enforces author/signer authorization, surfacing `Forbidden`/`NotFound` which
//! map straight to `403`/`404`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use relatum_domain::models::ids::ReportId;
use relatum_domain::models::report::ReviewDecision;
use relatum_domain::models::week::IsoWeek;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;
use relatum_domain::services::report::ReportForExport;
use relatum_export::{ReportDocument, SignatureBlock};

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

/// `GET /api/v1/reports/{id}/export` — render the report as a signed PDF
/// (Ausbildungsnachweis).
///
/// Deliberately **not** part of the typed OpenAPI surface: it returns a binary
/// `application/pdf` body, which OpenAPI → client codegen does not model cleanly (the
/// same reason signatures are base64-in-JSON). It is wired as a plain route in
/// [`router`](crate::routes::router), alongside the other non-JSON endpoints, and the
/// client downloads the bytes directly. Visible to exactly the callers who may
/// [`get`] the report; available in any state, stamping only the signatures on file.
pub async fn export<U, S, I, P, R, G>(
    State(state): State<AppState<U, S, I, P, R, G>>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
    G: SignatureStorage + Clone + 'static,
{
    let inputs = state
        .reports
        .export_inputs(user.id(), &ReportId::new(id))
        .await?;
    let week = *inputs.report.week();
    let document = to_export_document(inputs);
    let pdf = relatum_export::render_report_pdf(&document);

    let filename = format!("ausbildungsnachweis-{week}.pdf");
    let headers = [
        (header::CONTENT_TYPE, "application/pdf".to_owned()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    tracing::info!(report_id = %week, "report exported as pdf");
    Ok((headers, pdf).into_response())
}

/// Map the domain's [`ReportForExport`] onto the renderer's input. Signature image
/// bytes go in raw (no base64 — that is only an API/JSON concern). The author's date
/// is the end of the covered week; the signer's is when they signed.
fn to_export_document(inputs: ReportForExport) -> ReportDocument {
    let week = inputs.report.week();
    let week_range = format!(
        "{} bis {}",
        fmt_date_civil(week.monday()),
        fmt_date_civil(week.sunday())
    );

    let author = SignatureBlock {
        name: inputs.author_name,
        png_bytes: signature_bytes(inputs.author_signature.as_ref()),
        date: fmt_date_civil(week.sunday()),
    };
    let signer = inputs.signer.map(|s| SignatureBlock {
        name: s.name,
        png_bytes: signature_bytes(s.signature.as_ref()),
        date: fmt_timestamp(s.signed_at),
    });

    ReportDocument {
        author_name: author.name.clone(),
        department: inputs.report.department().as_str().to_owned(),
        week_range,
        report_no: Some(inputs.report.id().as_str().to_owned()),
        training_year: None,
        body_markdown: inputs.report.content().to_owned(),
        author,
        signer,
    }
}

fn signature_bytes(stored: Option<&relatum_domain::models::signature::StoredSignature>) -> Vec<u8> {
    stored
        .map(|s| s.signature.bytes().to_vec())
        .unwrap_or_default()
}

fn fmt_date_civil(date: jiff::civil::Date) -> String {
    date.strftime("%d.%m.%Y").to_string()
}

fn fmt_timestamp(ts: Timestamp) -> String {
    fmt_date_civil(ts.to_zoned(TimeZone::UTC).date())
}
