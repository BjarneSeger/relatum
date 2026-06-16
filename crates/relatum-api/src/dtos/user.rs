//! User-administration DTOs.
//!
//! Users are provisioned by the directory sync, not over the API, so there is no
//! registration body. The one manual mutation the API exposes is assigning a user
//! to a department, which is also what turns a regular directory user into a signer.

use relatum_domain::models::users::{DirectoryMarker, Role, User};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Body for assigning a user to a department.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssignDepartmentRequest {
    #[schema(example = "blue")]
    pub department: String,
}

/// The authenticated caller's own identity and effective role.
///
/// Returned by `GET /api/v1/me` so a client can pick the right UI without
/// re-deriving the role from endpoint behaviour. `role` is `null` while the user is
/// inert (no department assigned).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MeView {
    #[schema(example = "alice")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub role: Option<RoleDto>,
}

impl From<&User> for MeView {
    fn from(user: &User) -> Self {
        MeView {
            id: user.id().as_str().to_owned(),
            role: user.role().as_ref().map(RoleDto::from),
        }
    }
}

/// A single user as listed for the instructor's administration view.
///
/// Returned by `GET /api/v1/users`. Carries the directory `marker` as well as the
/// effective `role`, so the admin sees *what a user will become* once given a
/// department even while they are still inert (`role` is `null`, `department` is
/// `null`). The frontend keys its assign/clear actions off `id` but only ever shows
/// `username`, so no one has to handle a raw id.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserSummary {
    #[schema(example = "alice")]
    pub id: String,
    #[schema(example = "alice")]
    pub username: String,
    /// What the directory marks this user as, independent of any department.
    pub marker: MarkerDto,
    /// The department this user is assigned to, or `null` if inert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub department: Option<String>,
    /// The user's effective role, or `null` while they have no department.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub role: Option<RoleDto>,
}

impl From<&User> for UserSummary {
    fn from(user: &User) -> Self {
        UserSummary {
            id: user.id().as_str().to_owned(),
            username: user.username().to_owned(),
            marker: user.marker().into(),
            department: user.department().map(|d| d.as_str().to_owned()),
            role: user.role().as_ref().map(RoleDto::from),
        }
    }
}

/// What the directory's group membership marks a user as, on the wire.
///
/// Independent of any department: it is what a user *is* in the directory, which
/// (together with a department) determines their [`RoleDto`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkerDto {
    Instructor,
    Trainee,
    Regular,
}

impl From<&DirectoryMarker> for MarkerDto {
    fn from(marker: &DirectoryMarker) -> Self {
        match marker {
            DirectoryMarker::Instructor => MarkerDto::Instructor,
            DirectoryMarker::Trainee => MarkerDto::Trainee,
            DirectoryMarker::Regular => MarkerDto::Regular,
        }
    }
}

/// A user's effective role on the wire. Every active role carries the department it
/// is scoped to.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum RoleDto {
    Instructor { department: String },
    Trainee { department: String },
    Signer { department: String },
}

impl From<&Role> for RoleDto {
    fn from(role: &Role) -> Self {
        let department = role.department().as_str().to_owned();
        match role {
            Role::Instructor { .. } => RoleDto::Instructor { department },
            Role::Trainee { .. } => RoleDto::Trainee { department },
            Role::Signer { .. } => RoleDto::Signer { department },
        }
    }
}
