//! User-administration handlers: assign and clear a user's department.
//!
//! Users themselves are provisioned by the directory sync, not over the API. The
//! one manual mutation exposed here is department assignment — which is also what
//! activates a regular directory user as a signer. The domain
//! [`UserAdmin`](relatum_domain::services::admin::UserAdmin) carries no
//! authorization of its own, so these handlers gate on the caller being an
//! instructor.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use relatum_domain::models::ids::{DepartmentId, UserId};
use relatum_domain::models::users::{Role, User};
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::userstorage::UserStorage;

use crate::dtos::{AssignDepartmentRequest, UserSummary};
use crate::error::{ApiError, ErrorResponse};
use crate::extract::CurrentUser;
use crate::state::AppState;

/// Only instructors may administer users.
fn require_instructor(user: &User) -> Result<(), ApiError> {
    match user.role() {
        Some(Role::Instructor { .. }) => Ok(()),
        _ => Err(ApiError::Forbidden(
            "only instructors may administer users".into(),
        )),
    }
}

/// `GET /api/v1/users` — list every user (instructor-only).
///
/// Powers the admin view's user picker so a department can be assigned by username
/// rather than by a typed id. Read-only, but instructor-gated like the mutations
/// below since it exposes the full directory.
#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    operation_id = "list_users",
    responses(
        (status = 200, description = "Every user the instance knows about", body = [UserSummary]),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Caller is not an instructor", body = ErrorResponse),
    ),
)]
pub async fn list<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
    CurrentUser(actor): CurrentUser,
) -> Result<Json<Vec<UserSummary>>, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    require_instructor(&actor)?;
    let users = state.admin.list_users().await?;
    Ok(Json(users.iter().map(UserSummary::from).collect()))
}

/// `PUT /api/v1/users/{id}/department` — assign a user to a department.
#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/department",
    tag = "users",
    params(("id" = String, Path, description = "User id")),
    request_body = AssignDepartmentRequest,
    responses(
        (status = 204, description = "Department assigned"),
        (status = 400, description = "Unknown department", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Caller is not an instructor", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse),
    ),
)]
pub async fn assign_department<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
    CurrentUser(actor): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<AssignDepartmentRequest>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    require_instructor(&actor)?;
    let user_id = UserId::new(id);
    let department = DepartmentId::new(req.department);
    // The department label is a non-sensitive identifier; capture it before the value
    // is moved into the service call so it can be logged on success.
    let dept_label = department.as_str().to_owned();
    state.admin.assign_department(&user_id, department).await?;
    tracing::info!(
        actor = %actor.id().as_str(),
        target_user = %user_id.as_str(),
        department = %dept_label,
        "department assigned"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/users/{id}/department` — clear a user's department (rendering
/// them inert).
#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}/department",
    tag = "users",
    params(("id" = String, Path, description = "User id")),
    responses(
        (status = 204, description = "Department cleared"),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 403, description = "Caller is not an instructor", body = ErrorResponse),
        (status = 404, description = "User not found", body = ErrorResponse),
    ),
)]
pub async fn clear_department<U, S, I, P, R>(
    State(state): State<AppState<U, S, I, P, R>>,
    CurrentUser(actor): CurrentUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError>
where
    U: UserStorage + Clone + 'static,
    S: SessionRepository + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + Clone + 'static,
{
    require_instructor(&actor)?;
    let user_id = UserId::new(id);
    state.admin.clear_department(&user_id).await?;
    tracing::info!(
        actor = %actor.id().as_str(),
        target_user = %user_id.as_str(),
        "department cleared"
    );
    Ok(StatusCode::NO_CONTENT)
}
