//! HTTP request handlers — **scaffold**.
//!
//! Handlers are the thin adapter between the wire and the domain. Each one should
//! stay tiny (ideally under ~20 lines) and follow the same shape:
//!
//! 1. extract the request (path/query/body → a [`crate::dtos`] type),
//! 2. call the relevant domain service
//!    (e.g. [`Authenticator`](relatum_domain::services::auth::Authenticator) or
//!    [`MetaService`](relatum_domain::services::meta::MetaService)) obtained from
//!    [`AppState`](crate::state::AppState),
//! 3. map the result: a domain model → a DTO (via `From`), or a
//!    [`DomainError`](relatum_domain::DomainError) →
//!    [`ApiError`](crate::ApiError) (via `ApiError::from`) → HTTP response.
//!
//! No business logic lives here — each handler is generic over the five ports the
//! [`AppState`](crate::state::AppState) carries and simply forwards to a domain
//! service. Each handler carries its own `#[utoipa::path]`, and
//! [`crate::routes::api_router`] registers it via `utoipa-axum`, so the served
//! route and its OpenAPI operation come from one place.

pub mod auth;
pub mod meta;
pub mod reports;
pub mod sso;
pub mod users;
