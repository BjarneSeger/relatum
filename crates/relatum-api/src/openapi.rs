//! The base OpenAPI document: API metadata and tags.
//!
//! The **paths and schemas are not listed here** — they are collected from the
//! actual routes by [`crate::routes::api_router`] (via `utoipa-axum`), so the spec
//! can't drift from the handlers. This type only seeds the document with the
//! title, description, license, and tag descriptions; obtain the full document
//! with [`crate::routes::openapi`].

use utoipa::OpenApi;

/// The base OpenAPI document (metadata only). Build the full spec with
/// [`crate::routes::openapi`].
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Relatum API",
        description = "Contract for the Relatum weekly-report service.",
        license(name = "MIT OR Apache-2.0"),
    ),
    tags(
        (name = "meta", description = "Service metadata and health."),
        (name = "auth", description = "Session-based authentication."),
        (name = "reports", description = "The trainee to instructor report workflow."),
        (name = "signatures", description = "Per-user signature images for report sign-off."),
        (name = "users", description = "User management (instructor-only)."),
    ),
)]
pub struct ApiDoc;
