//! `relatum-api` — the transport/contract layer for Relatum.
//!
//! This crate turns the domain contracts in `relatum-domain` into an HTTP API. It
//! depends only on the domain (never on a concrete backend such as
//! `relatum-infra`), so the wire format and the storage technology evolve
//! independently. The dependency arrow points inward: `relatum-api → relatum-domain`.
//!
//! - [`dtos`] — the serde + [`utoipa::ToSchema`] data-transfer types that cross
//!   the wire as JSON, with `From` conversions from the domain models. They are
//!   deliberately separate from the domain types so the contract stays stable.
//! - [`error`] — a unified [`ApiError`](error::ApiError) plus the JSON
//!   [`ErrorResponse`](error::ErrorResponse) body shape. `ApiError` maps from
//!   [`DomainError`](relatum_domain::DomainError) and exposes its HTTP status as a
//!   plain `u16`, so this crate never needs a web framework just to model errors.
//! - [`openapi`] — the base [`ApiDoc`](openapi::ApiDoc) (title/tags only). The full
//!   OpenAPI document — paths *and* schemas — is assembled from the live routes by
//!   [`routes::openapi`] (via `utoipa-axum`), so it can never drift from the
//!   handlers. `examples/export-openapi.rs` writes it for `openapi-generator`.
//! - [`handlers`], [`routes`], [`state`], [`extract`] — the axum HTTP layer.
//!   [`state::AppState`] holds the concrete domain services (generic over the
//!   outbound ports); [`extract`] turns a bearer token into the acting user; the
//!   [`handlers`] forward to the services (each carrying its `#[utoipa::path]`); and
//!   [`routes::router`] mounts them all under `/api/v1`.

pub mod dtos;
pub mod error;
pub mod extract;
pub mod handlers;
pub mod openapi;
pub mod routes;
pub mod spec;
pub mod state;

pub use error::{ApiError, ErrorResponse};
pub use openapi::ApiDoc;
pub use routes::{api_router, router};
pub use spec::{openapi_doc, openapi_json};
pub use state::AppState;
