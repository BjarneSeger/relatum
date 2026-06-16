//! The report workflow: the role-aware dashboard, a report's detail page, and the
//! create / revise / submit / review actions, plus the htmx markdown preview.
//!
//! The dashboard's *content* varies by role (the API decides what `list_reports`
//! returns), but the page is the same. State-changing actions answer with a swapped
//! table row when called from htmx and fall back to a redirect (full reload) for a
//! plain `<form>` submit.

use axum::extract::{Form, Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use relatum_client::ReviewDecisionDto;
use serde::Deserialize;

use crate::error::WebError;
use crate::handlers::is_htmx;
use crate::markdown;
use crate::state::WebState;
use crate::view::{self, Dashboard, NewReport, Preview, ReportPage, RoleKind, Theme, Viewer};
use askama::Template;

/// `GET /` — the caller's dashboard. An inert user (no department) sees a notice
/// rather than the report list, which would otherwise be `403`.
pub async fn dashboard(
    State(state): State<WebState>,
    jar: CookieJar,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let me = client.me().await?;
    let viewer = Viewer::of(&me);
    let can_create = viewer.is_trainee();
    let is_instructor = viewer.is_instructor();

    let Some(kind) = viewer.kind else {
        let page = Dashboard {
            theme: Theme::from_cookie(&jar),
            viewer_id: viewer.id,
            role_label: viewer.label,
            inert: true,
            can_create: false,
            is_instructor: false,
            heading: String::new(),
            rows: Vec::new(),
        };
        return Ok(Html(page.render()?).into_response());
    };

    let heading = match kind {
        RoleKind::Trainee => "Your reports".to_owned(),
        RoleKind::Signer => match &viewer.department {
            Some(dept) => format!("{dept} queue"),
            None => "Queue".to_owned(),
        },
        RoleKind::Instructor => "All reports".to_owned(),
    };

    let reports = client.list_reports().await?;
    let rows = reports
        .iter()
        .map(|report| view::render_row(kind, report))
        .collect::<Result<Vec<_>, _>>()?;

    let page = Dashboard {
        theme: Theme::from_cookie(&jar),
        viewer_id: viewer.id,
        role_label: viewer.label,
        inert: false,
        can_create,
        is_instructor,
        heading,
        rows,
    };
    Ok(Html(page.render()?).into_response())
}

/// `GET /reports/new` — the draft-creation form (trainee-facing; the API enforces it).
pub async fn new_page(State(state): State<WebState>, jar: CookieJar) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let me = client.me().await?;
    let today = Timestamp::now().to_zoned(TimeZone::UTC).date();
    let page = NewReport {
        theme: Theme::from_cookie(&jar),
        viewer_id: Viewer::of(&me).id,
        today: today.strftime("%Y-%m-%d").to_string(),
    };
    Ok(Html(page.render()?).into_response())
}

#[derive(Deserialize)]
pub struct CreateForm {
    /// Any day in the target week (`YYYY-MM-DD`, from the native date picker); the
    /// week the report covers is derived from it.
    day: String,
    content: String,
}

/// `POST /reports` — create a draft and go to its page.
pub async fn create(
    State(state): State<WebState>,
    jar: CookieJar,
    Form(form): Form<CreateForm>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let week = iso_week_of(form.day.trim())?;
    let id = client.create_report(&week, &form.content).await?;
    Ok(Redirect::to(&format!("/reports/{id}")).into_response())
}

/// Map a `YYYY-MM-DD` calendar day to the canonical `YYYY-Www` of the ISO week it
/// falls in — the form lets a trainee pick any day, but a report is filed per week.
fn iso_week_of(day: &str) -> Result<String, WebError> {
    let date: Date = day.parse().map_err(|_| WebError::Api {
        status: 400,
        message: format!("not a valid date: {day:?}"),
    })?;
    let wd = date.iso_week_date();
    Ok(format!("{:04}-W{:02}", wd.year(), wd.week()))
}

/// `GET /reports/{id}` — a single report, with the controls the viewer is allowed.
pub async fn detail(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let me = client.me().await?;
    let viewer = Viewer::of(&me);
    let report = client.get_report(&id).await?;

    let is_author = viewer.id == report.author;
    let can_edit = is_author && !view::is_signed(&report.status);
    let can_submit = is_author && view::is_submittable(&report.status);
    let can_review = viewer.kind == Some(RoleKind::Signer) && view::is_submitted(&report.status);

    let page = ReportPage {
        theme: Theme::from_cookie(&jar),
        viewer_id: viewer.id,
        id: report.id.clone(),
        author: report.author.clone(),
        department: report.department.clone(),
        week: report.week.clone(),
        status_label: view::status_label(&report.status),
        status_class: view::status_class(&report.status),
        body_html: markdown::to_safe_html(&report.content),
        content: report.content.clone(),
        can_edit,
        can_submit,
        can_review,
    };
    Ok(Html(page.render()?).into_response())
}

#[derive(Deserialize)]
pub struct ReviseForm {
    content: String,
}

/// `POST /reports/{id}/revise` — replace a draft/rejected/submitted body.
pub async fn revise(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Form(form): Form<ReviseForm>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    client.revise_report(&id, &form.content).await?;
    Ok(Redirect::to(&format!("/reports/{id}")).into_response())
}

/// `POST /reports/{id}/submit` — submit into the department queue. From htmx, swap the
/// updated row; otherwise redirect to the report.
pub async fn submit(
    State(state): State<WebState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    client.submit_report(&id).await?;
    if is_htmx(&headers) {
        let report = client.get_report(&id).await?;
        // Only a trainee (the author) can reach Submit, so render the row as one.
        Ok(Html(view::render_row(RoleKind::Trainee, &report)?).into_response())
    } else {
        Ok(Redirect::to(&format!("/reports/{id}")).into_response())
    }
}

#[derive(Deserialize)]
pub struct ReviewForm {
    decision: String,
    reason: Option<String>,
}

/// `POST /reports/{id}/review` — sign or reject. The dashboard's inline Sign is htmx
/// (swap the row); the detail page's Sign/Reject forms are plain (redirect home).
pub async fn review(
    State(state): State<WebState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<ReviewForm>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let decision = match form.decision.as_str() {
        "sign" => ReviewDecisionDto::Sign,
        "reject" => ReviewDecisionDto::Reject(form.reason.unwrap_or_default()),
        other => {
            return Err(WebError::Api {
                status: 400,
                message: format!("unknown review decision '{other}'"),
            });
        }
    };
    client.review_report(&id, decision).await?;

    if is_htmx(&headers) {
        let report = client.get_report(&id).await?;
        // Only a signer can reach review, so render the row from a signer's view.
        Ok(Html(view::render_row(RoleKind::Signer, &report)?).into_response())
    } else {
        Ok(Redirect::to("/").into_response())
    }
}

#[derive(Deserialize)]
pub struct PreviewForm {
    content: String,
}

/// `POST /preview` — render markdown to sanitized HTML for the live editor preview.
pub async fn preview(
    State(state): State<WebState>,
    jar: CookieJar,
    Form(form): Form<PreviewForm>,
) -> Result<Response, WebError> {
    // Require a session so the preview endpoint is not an open markdown renderer.
    state.authed(&jar)?;
    let page = Preview {
        body_html: markdown::to_safe_html(&form.content),
    };
    Ok(Html(page.render()?).into_response())
}
