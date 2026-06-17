//! The signature settings page: register (or replace) the caller's signature.
//!
//! A trainee or signer must have a signature on file before they can submit or sign a
//! report. The browser draws it on a `<canvas>` (or loads a PNG into it) and posts the
//! canvas as a base64 PNG; this handler forwards it to the API. Keeping the encode in
//! the browser and the decode here means the public API stays JSON-only — no binary
//! body ever crosses it.

use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use relatum_client::SignatureFormatDto;
use serde::Deserialize;

use crate::error::WebError;
use crate::state::WebState;
use crate::view::{SignaturePage, Theme, Viewer};
use askama::Template;

/// `GET /settings/signature` — the capture pad plus the current signature's status.
pub async fn page(State(state): State<WebState>, jar: CookieJar) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let me = client.me().await?;
    let viewer = Viewer::of(&me);
    let current = client.get_signature().await?;
    let updated_note = current
        .as_ref()
        .map(|s| format!("last set {}", s.updated_at))
        .unwrap_or_default();
    let page = SignaturePage {
        theme: Theme::from_cookie(&jar),
        can_register: viewer.signs_reports(),
        viewer_id: viewer.id,
        has_signature: current.is_some(),
        updated_note,
    };
    Ok(Html(page.render()?).into_response())
}

#[derive(Deserialize)]
pub struct SetForm {
    /// Base64 PNG produced by the browser (the `data:` URL prefix already stripped).
    data_base64: String,
}

/// `POST /settings/signature` — store the posted PNG as the caller's signature.
pub async fn set(
    State(state): State<WebState>,
    jar: CookieJar,
    Form(form): Form<SetForm>,
) -> Result<Response, WebError> {
    let client = state.authed(&jar)?;
    let bytes = BASE64
        .decode(form.data_base64.trim().as_bytes())
        .map_err(|_| WebError::Api {
            status: 400,
            message: "the signature image was not valid base64".to_owned(),
        })?;
    // The client re-encodes to base64 for the JSON API; the server validates the PNG
    // magic bytes and size, so a bogus upload comes back as a 400 here.
    client
        .set_signature(SignatureFormatDto::Png, &bytes)
        .await?;
    Ok(Redirect::to("/settings/signature").into_response())
}
