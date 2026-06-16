//! Instructor user administration: list users and assign/clear their department.
//!
//! The whole point of this page is that nobody types a user id. The table shows
//! usernames; each row's assign/clear form carries the id in its action URL, never in
//! a field a human fills in. The API gates every call on the caller being an
//! instructor (a non-instructor gets `403`, surfaced as an error page).

use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use serde::Deserialize;

use crate::error::WebError;
use crate::state::WebState;
use crate::view::{self, Admin, Theme, Viewer};
use askama::Template;

/// `GET /admin` — the user table with a department picker per row.
pub async fn page(State(state): State<WebState>, jar: CookieJar) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let me = client.me().await?;
    // `list_users` is instructor-only on the server; a non-instructor caller gets a
    // 403 here, which renders as an access-denied page.
    let users = client.list_users().await?;

    // Offer the configured departments plus any already in use, so existing
    // assignments are always representable even if config drifts.
    let mut departments = state.departments.clone();
    for user in &users {
        if let Some(dept) = &user.department
            && !departments.contains(dept)
        {
            departments.push(dept.clone());
        }
    }

    let rows = users
        .iter()
        .map(|user| view::render_user_row(user, &departments))
        .collect::<Result<Vec<_>, _>>()?;

    let page = Admin {
        theme: Theme::from_cookie(&jar),
        viewer_id: Viewer::of(&me).id,
        rows,
    };
    Ok(Html(page.render()?).into_response())
}

#[derive(Deserialize)]
pub struct AssignForm {
    department: String,
}

/// `POST /admin/users/{id}/department` — assign a department (activates a signer).
pub async fn assign(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Form(form): Form<AssignForm>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    client.assign_department(&id, form.department.trim()).await?;
    Ok(Redirect::to("/admin").into_response())
}

/// `POST /admin/users/{id}/department/clear` — clear a department (renders inert).
pub async fn clear(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    client.clear_department(&id).await?;
    Ok(Redirect::to("/admin").into_response())
}
