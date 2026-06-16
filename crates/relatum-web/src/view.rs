//! View models, askama templates, and the helpers that render them.
//!
//! The handlers stay thin by pushing all presentation here: classifying the caller's
//! role, turning wire DTOs into display rows, and pre-rendering list rows to HTML
//! strings. Rendering rows in Rust (rather than `{% include %}`-ing inside a loop)
//! lets the very same row template serve both the full page and the htmx fragment a
//! single action swaps in, with no duplicated markup.

use askama::Template;
use axum_extra::extract::CookieJar;
use relatum_client::{MarkerDto, MeView, ReportView, ReviewStatusDto, RoleDto, UserSummary};

use crate::session::THEME_COOKIE;

// ----------------------------------------------------------------------------
// Colour theme.
// ----------------------------------------------------------------------------

/// The viewer's chosen colour theme. `Auto` (the default) defers to the OS via the
/// CSS `prefers-color-scheme` media query; `Light`/`Dark` force a palette. The value
/// is rendered as `<html data-theme="…">` and the stylesheet keys off it.
#[derive(Clone, Copy)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    /// The `data-theme` attribute value (also the cookie value).
    pub fn attr(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Theme::Auto)
    }

    pub fn is_light(self) -> bool {
        matches!(self, Theme::Light)
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }

    /// Parse a cookie/form value; anything unrecognised falls back to `Auto`.
    pub fn parse(value: &str) -> Self {
        match value {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::Auto,
        }
    }

    /// The theme carried by the request's cookie (absent → `Auto`).
    pub fn from_cookie(jar: &CookieJar) -> Self {
        jar.get(THEME_COOKIE)
            .map(|c| Theme::parse(c.value()))
            .unwrap_or(Theme::Auto)
    }
}

// ----------------------------------------------------------------------------
// Role classification.
// ----------------------------------------------------------------------------

/// The active role a viewer acts as. Mirrors the populated [`RoleDto`] variants; an
/// inert user (no department) has none.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    Trainee,
    Signer,
    Instructor,
}

/// The caller's classified role plus a display label and their department.
pub struct Viewer {
    pub id: String,
    pub kind: Option<RoleKind>,
    pub label: String,
    pub department: Option<String>,
}

impl Viewer {
    /// Classify the authenticated caller from their `/me` view.
    pub fn of(me: &MeView) -> Self {
        let id = me.id.clone();
        match &me.role {
            None => Viewer {
                id,
                kind: None,
                label: "No department assigned".to_owned(),
                department: None,
            },
            Some(RoleDto::Trainee(dept)) => Viewer {
                id,
                kind: Some(RoleKind::Trainee),
                label: format!("Trainee · {dept}"),
                department: Some(dept.clone()),
            },
            Some(RoleDto::Signer(dept)) => Viewer {
                id,
                kind: Some(RoleKind::Signer),
                label: format!("Signer · {dept}"),
                department: Some(dept.clone()),
            },
            Some(RoleDto::Instructor(dept)) => Viewer {
                id,
                kind: Some(RoleKind::Instructor),
                label: format!("Instructor · {dept}"),
                department: Some(dept.clone()),
            },
        }
    }

    pub fn is_trainee(&self) -> bool {
        self.kind == Some(RoleKind::Trainee)
    }

    pub fn is_instructor(&self) -> bool {
        self.kind == Some(RoleKind::Instructor)
    }
}

// ----------------------------------------------------------------------------
// Report status helpers.
// ----------------------------------------------------------------------------

/// A one-line human label for a report's status (carries the signer / reason).
pub fn status_label(status: &ReviewStatusDto) -> String {
    match status {
        ReviewStatusDto::Draft => "Draft".to_owned(),
        ReviewStatusDto::Submitted { .. } => "Submitted".to_owned(),
        ReviewStatusDto::Signed { by, .. } => format!("Signed by {by}"),
        ReviewStatusDto::Rejected { reason, .. } => format!("Rejected: {reason}"),
    }
}

/// A CSS class keying the status badge's colour.
pub fn status_class(status: &ReviewStatusDto) -> &'static str {
    match status {
        ReviewStatusDto::Draft => "draft",
        ReviewStatusDto::Submitted { .. } => "submitted",
        ReviewStatusDto::Signed { .. } => "signed",
        ReviewStatusDto::Rejected { .. } => "rejected",
    }
}

/// Whether the report may be submitted (a fresh draft or a rejected one).
pub fn is_submittable(status: &ReviewStatusDto) -> bool {
    matches!(
        status,
        ReviewStatusDto::Draft | ReviewStatusDto::Rejected { .. }
    )
}

/// Whether the report is awaiting a signer's verdict.
pub fn is_submitted(status: &ReviewStatusDto) -> bool {
    matches!(status, ReviewStatusDto::Submitted { .. })
}

/// Whether the report has been signed (and so is frozen).
pub fn is_signed(status: &ReviewStatusDto) -> bool {
    matches!(status, ReviewStatusDto::Signed { .. })
}

// ----------------------------------------------------------------------------
// Report view models + templates.
// ----------------------------------------------------------------------------

/// A report row shaped for display.
pub struct ReportVm {
    pub id: String,
    pub author: String,
    pub week: String,
    pub status_label: String,
    pub status_class: &'static str,
}

impl ReportVm {
    pub fn of(report: &ReportView) -> Self {
        ReportVm {
            id: report.id.clone(),
            author: report.author.clone(),
            week: report.week.clone(),
            status_label: status_label(&report.status),
            status_class: status_class(&report.status),
        }
    }
}

/// One row of a report table; also the htmx fragment a sign/submit action swaps in.
#[derive(Template)]
#[template(path = "_report_row.html")]
pub struct ReportRow {
    pub r: ReportVm,
    /// Trainee viewing a submittable report → offer an inline Submit.
    pub show_submit: bool,
    /// Signer viewing a submitted report → offer an inline Sign.
    pub show_sign: bool,
}

/// Render a single report row from the viewer's role.
pub fn render_row(kind: RoleKind, report: &ReportView) -> Result<String, askama::Error> {
    let show_submit = kind == RoleKind::Trainee && is_submittable(&report.status);
    let show_sign = kind == RoleKind::Signer && is_submitted(&report.status);
    ReportRow {
        r: ReportVm::of(report),
        show_submit,
        show_sign,
    }
    .render()
}

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct Dashboard {
    pub theme: Theme,
    pub viewer_id: String,
    pub role_label: String,
    /// No department assigned: show a notice instead of a (forbidden) report list.
    pub inert: bool,
    pub can_create: bool,
    pub is_instructor: bool,
    pub heading: String,
    pub rows: Vec<String>,
}

#[derive(Template)]
#[template(path = "report.html")]
pub struct ReportPage {
    pub theme: Theme,
    pub viewer_id: String,
    pub id: String,
    pub author: String,
    pub department: String,
    pub week: String,
    pub status_label: String,
    pub status_class: &'static str,
    pub body_html: String,
    pub content: String,
    /// Author may edit (revise) — anything but a signed report.
    pub can_edit: bool,
    /// Author may submit — a draft or rejected report.
    pub can_submit: bool,
    /// Signer in this department may sign or reject a submitted report.
    pub can_review: bool,
}

#[derive(Template)]
#[template(path = "report_new.html")]
pub struct NewReport {
    pub theme: Theme,
    pub viewer_id: String,
    /// Today as `YYYY-MM-DD`, pre-filling the date picker to the current week.
    pub today: String,
}

#[derive(Template)]
#[template(path = "_preview.html")]
pub struct Preview {
    pub body_html: String,
}

// ----------------------------------------------------------------------------
// Auth + admin templates.
// ----------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "login.html")]
pub struct Login {
    pub theme: Theme,
    pub sso_enabled: bool,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin.html")]
pub struct Admin {
    pub theme: Theme,
    pub viewer_id: String,
    pub rows: Vec<String>,
}

/// A department choice in the assignment dropdown.
pub struct DeptOption {
    pub name: String,
    pub selected: bool,
}

/// One row of the admin user table (also a self-contained fragment).
#[derive(Template)]
#[template(path = "_user_row.html")]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub marker: &'static str,
    pub department_label: String,
    pub role_label: String,
    pub has_department: bool,
    /// No departments to offer → fall back to a free-text field.
    pub free_text: bool,
    pub departments: Vec<DeptOption>,
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    pub theme: Theme,
    pub status: u16,
    pub message: String,
}

/// Human label for a directory marker.
pub fn marker_text(marker: &MarkerDto) -> &'static str {
    match marker {
        MarkerDto::Instructor => "Instructor",
        MarkerDto::Trainee => "Trainee",
        MarkerDto::Regular => "Regular",
    }
}

/// Human label for an effective role (or "inert" for a department-less user).
pub fn role_label(role: &Option<RoleDto>) -> String {
    match role {
        None => "inert".to_owned(),
        Some(RoleDto::Instructor(dept)) => format!("Instructor · {dept}"),
        Some(RoleDto::Trainee(dept)) => format!("Trainee · {dept}"),
        Some(RoleDto::Signer(dept)) => format!("Signer · {dept}"),
    }
}

/// Render one admin user row, with the dropdown pre-selecting the current department.
pub fn render_user_row(
    user: &UserSummary,
    departments: &[String],
) -> Result<String, askama::Error> {
    let options = departments
        .iter()
        .map(|name| DeptOption {
            name: name.clone(),
            selected: user.department.as_deref() == Some(name.as_str()),
        })
        .collect();
    UserRow {
        id: user.id.clone(),
        username: user.username.clone(),
        marker: marker_text(&user.marker),
        department_label: user.department.clone().unwrap_or_else(|| "—".to_owned()),
        role_label: role_label(&user.role),
        has_department: user.department.is_some(),
        free_text: departments.is_empty(),
        departments: options,
    }
    .render()
}
