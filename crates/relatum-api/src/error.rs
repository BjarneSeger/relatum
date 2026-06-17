//! Unified API error type and its wire representation.
//!
//! [`ApiError`] is the transport-side counterpart of
//! [`DomainError`](relatum_domain::DomainError): it adds an HTTP status (exposed
//! as a plain `u16` via [`ApiError::status`], so the eventual web framework can
//! translate it without this type depending on axum/hyper) and a stable,
//! machine-readable code. The JSON body clients receive is [`ErrorResponse`].

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use relatum_domain::DomainError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

/// Everything the API can fail with.
///
/// Each variant maps to an HTTP status via [`ApiError::status`], and its
/// `Display` text becomes the client-facing [`ErrorResponse::message`]. Build one
/// from a [`DomainError`] at the handler boundary (`ApiError::from(err)`).
#[derive(Debug, Error)]
pub enum ApiError {
    /// The requested resource does not exist. -> 404
    #[error("not found: {0}")]
    NotFound(String),

    /// The request was malformed or failed validation. -> 400
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Authentication is required or the token is invalid. -> 401
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// The user is not allowed to access this -> 403
    #[error("access denied: {0}")]
    Forbidden(String),

    /// The request conflicts with existing state. -> 409
    #[error("conflict: {0}")]
    Conflict(String),

    /// The action is permitted but a required precondition is unmet (e.g. the caller
    /// must register a signature before submitting or signing a report). -> 428
    #[error("precondition required: {0}")]
    PreconditionRequired(String),

    /// A backend operation (storage, the user directory, the session store, …) failed. -> 502
    #[error("backend error: {0}")]
    Backend(String),

    /// An unexpected internal error. -> 500
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    /// The HTTP status code this error maps to.
    ///
    /// Returned as a plain `u16` so the server can build a response with whatever
    /// framework it likes (e.g. `StatusCode::from_u16(err.status())`).
    pub fn status(&self) -> u16 {
        match self {
            ApiError::NotFound(_) => 404,
            ApiError::BadRequest(_) => 400,
            ApiError::Unauthorized(_) => 401,
            ApiError::Forbidden(_) => 403,
            ApiError::Conflict(_) => 409,
            ApiError::PreconditionRequired(_) => 428,
            ApiError::Backend(_) => 502,
            ApiError::Internal(_) => 500,
        }
    }

    pub fn code(&self) -> String {
        match self {
            ApiError::Backend(_) => "backend".into(),
            ApiError::BadRequest(_) => "bad_request".into(),
            ApiError::Conflict(_) => "conflict".into(),
            ApiError::Forbidden(_) => "forbidden".into(),
            ApiError::Internal(_) => "internal".into(),
            ApiError::NotFound(_) => "not_found".into(),
            ApiError::PreconditionRequired(_) => "precondition_required".into(),
            ApiError::Unauthorized(_) => "unauthorized".into(),
        }
    }
}

/// Translate a domain failure into its transport representation.
///
/// This is the one place the API's status-code policy is decided.
impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::NotFound(m) => ApiError::NotFound(m),
            DomainError::Invalid(m) => ApiError::BadRequest(m),
            DomainError::Unauthorized(m) => ApiError::Unauthorized(m),
            DomainError::Forbidden(m) => ApiError::Forbidden(m),
            DomainError::Conflict(m) => ApiError::Conflict(m),
            DomainError::Precondition(m) => ApiError::PreconditionRequired(m),
            DomainError::Backend(m) => ApiError::Backend(m),
            DomainError::Internal(m) => ApiError::Internal(m),
        }
    }
}

/// The JSON body returned to clients when a request fails.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// HTTP status code, echoed in the body for convenience.
    #[schema(example = 404)]
    pub status: u16,
    /// Stable machine-readable code, e.g. `"not_found"`.
    #[schema(example = "not_found")]
    pub code: String,
    /// Human-readable description of what went wrong.
    #[schema(example = "not found: summary for 2026-W22")]
    pub message: String,
}

impl From<&ApiError> for ErrorResponse {
    fn from(err: &ApiError) -> Self {
        ErrorResponse {
            status: err.status(),
            code: err.code(),
            message: err.to_string(),
        }
    }
}

impl From<ApiError> for ErrorResponse {
    fn from(err: ApiError) -> Self {
        (&err).into()
    }
}

/// Render an [`ApiError`] as an HTTP response: the mapped status plus the
/// [`ErrorResponse`] JSON body. This is the single place handlers' `?`-propagated
/// errors become responses, so every failure path shares one wire shape.
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        // Mirror relatum-web: a 5xx is internal diagnostics the client never sees, so
        // log it at error. 4xx are caller-facing and expected (validation, auth and
        // permission denials), so log at debug — visible when debugging without
        // spamming info. The `Display` text is the sanitized domain/handler message;
        // it carries no token or secret. Runs inside the request span, so each line is
        // correlated with the method+path.
        if status.is_server_error() {
            tracing::error!(
                status = status.as_u16(),
                code = %self.code(),
                message = %self,
                "request failed (server error)"
            );
        } else {
            tracing::debug!(
                status = status.as_u16(),
                code = %self.code(),
                message = %self,
                "request rejected (client error)"
            );
        }

        (status, Json(ErrorResponse::from(&self))).into_response()
    }
}
