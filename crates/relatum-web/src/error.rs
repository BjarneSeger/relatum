//! The web layer's error type and how it becomes a response.

use askama::Template;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use relatum_client::ClientError;

use crate::session::SESSION_COOKIE;
use crate::view::{ErrorPage, Theme};

/// Everything a handler can fail with, and how the browser should see it.
pub enum WebError {
    /// No (or no-longer-valid) session: bounce to `/login`, clearing the cookie.
    NeedsLogin,
    /// The API answered with a structured error — render it as an error page under
    /// the same status.
    Api { status: u16, message: String },
    /// Something went wrong on our side (template render, an unexpected API shape).
    Internal(String),
}

impl From<ClientError> for WebError {
    fn from(err: ClientError) -> Self {
        match err {
            // An expired/invalid session reads the same as not being logged in.
            ClientError::Api { status: 401, .. } => WebError::NeedsLogin,
            ClientError::Api { status, message, .. } => WebError::Api { status, message },
            ClientError::Transport(detail) => WebError::Api {
                status: 502,
                message: format!("could not reach the API: {detail}"),
            },
            ClientError::Unexpected(detail) => WebError::Internal(detail),
        }
    }
}

impl From<askama::Error> for WebError {
    fn from(err: askama::Error) -> Self {
        WebError::Internal(format!("template error: {err}"))
    }
}

/// Shown to the user for any 5xx. The diagnostic detail goes to the log, never the
/// page, so an internal API host or a raw upstream body can't leak to the browser.
const GENERIC_5XX: &str = "The service is temporarily unavailable. Please try again.";

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        match self {
            WebError::NeedsLogin => {
                // Drop the useless session cookie and send the browser to login.
                let clear =
                    format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax");
                (
                    StatusCode::SEE_OTHER,
                    [
                        (header::LOCATION, "/login".to_owned()),
                        (header::SET_COOKIE, clear),
                    ],
                )
                    .into_response()
            }
            WebError::Api { status, message } => {
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                // 4xx carry a user-meaningful domain message (e.g. "a report already
                // exists for this week"); 5xx detail is internal diagnostics — log it
                // and show the user a generic line instead.
                if code.is_server_error() {
                    tracing::error!(status = code.as_u16(), %message, "upstream/server error");
                    error_page(code, GENERIC_5XX)
                } else {
                    error_page(code, &message)
                }
            }
            WebError::Internal(message) => {
                tracing::error!(%message, "internal web error");
                error_page(StatusCode::INTERNAL_SERVER_ERROR, GENERIC_5XX)
            }
        }
    }
}

/// Render the error template under `status`, degrading to plain text if even the
/// template fails.
fn error_page(status: StatusCode, message: &str) -> Response {
    // No cookie jar reaches the error path, so error pages default to Auto (which
    // still honours the OS dark preference); the explicit pick is not threaded here.
    let body = ErrorPage {
        theme: Theme::Auto,
        status: status.as_u16(),
        message: message.to_owned(),
    }
    .render()
    .unwrap_or_else(|_| format!("error {}: {message}", status.as_u16()));
    (status, Html(body)).into_response()
}
