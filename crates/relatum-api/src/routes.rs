//! Router composition — the single source of truth for both the live routes and
//! the OpenAPI document.
//!
//! Every operation is registered once, via [`utoipa_axum::routes!`], from the
//! `#[utoipa::path]` attribute on the **real handler**. The HTTP method and path
//! are taken from that attribute, so the served routes and the generated spec
//! cannot disagree. [`api_router`] is the shared builder; [`router`] applies state
//! for serving and [`openapi`] extracts the document.
//!
//! This is the one place the five port generics are bounded for axum (`Clone +
//! 'static`; `Send + Sync` come from the ports' supertraits).

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response};
use tower_http::classify::ServerErrorsFailureClass;
use tower_http::trace::TraceLayer;
use tracing::{Span, field};

use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::SSOProvider;
use relatum_domain::ports::status::StatusBackend;
use relatum_domain::ports::userstorage::UserStorage;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::ApiDoc;
use crate::handlers;
use crate::state::AppState;
use utoipa::OpenApi as _;

/// Build the combined route + OpenAPI router. Handlers sharing a path (e.g. the
/// `POST`/`GET` on `/reports`) are grouped in one `routes!` so they merge into a
/// single path entry. Schemas referenced by the `#[utoipa::path]` responses are
/// collected automatically.
pub fn api_router<U, S, I, P, R, G>() -> OpenApiRouter<AppState<U, S, I, P, R, G>>
where
    U: UserStorage + StatusBackend + Clone + 'static,
    S: SessionRepository + StatusBackend + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        // Service metadata & health.
        .routes(routes!(handlers::meta::info))
        .routes(routes!(handlers::meta::healthz))
        .routes(routes!(handlers::meta::readyz))
        // Authentication.
        .routes(routes!(handlers::auth::login))
        .routes(routes!(handlers::auth::logout))
        .routes(routes!(handlers::auth::refresh))
        .routes(routes!(handlers::auth::me))
        // SSO availability + back-channel handoff exchange. The browser-facing
        // start/callback are plain routes added in `router` (they redirect rather than
        // return JSON, so they stay out of the OpenAPI document and the generated
        // client); `exchange` returns JSON, so it is typed.
        .routes(routes!(handlers::sso::info))
        .routes(routes!(handlers::sso::exchange))
        // Reports.
        .routes(routes!(handlers::reports::create, handlers::reports::list))
        .routes(routes!(handlers::reports::get, handlers::reports::revise))
        .routes(routes!(handlers::reports::submit))
        .routes(routes!(handlers::reports::review))
        // Self-service signatures: set/replace + get the caller's own.
        .routes(routes!(handlers::signature::set, handlers::signature::get))
        // User administration (instructor-only): listing + department assignment.
        .routes(routes!(handlers::users::list))
        .routes(routes!(
            handlers::users::assign_department,
            handlers::users::clear_department
        ))
}

/// Build the application router from a fully-wired [`AppState`], ready to serve.
pub fn router<U, S, I, P, R, G>(state: AppState<U, S, I, P, R, G>) -> Router
where
    U: UserStorage + StatusBackend + Clone + 'static,
    S: SessionRepository + StatusBackend + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    let (router, _) = api_router::<U, S, I, P, R, G>().split_for_parts();
    router
        // Browser-facing SSO redirects: not JSON, so kept off the typed API surface.
        .route(
            "/api/v1/auth/sso/start",
            axum::routing::get(handlers::sso::start::<U, S, I, P, R, G>),
        )
        .route(
            "/api/v1/auth/sso/callback",
            axum::routing::get(handlers::sso::callback::<U, S, I, P, R, G>),
        )
        .with_state(state)
        // Per-request span. Applied last, to the state-erased `Router`, so the layer
        // stays clear of the five port generics. The span carries only the method and
        // **path** — never the full URI, since the SSO start/callback query strings
        // carry `code`/`state`. `status`/`latency_ms` are recorded on response; the
        // span itself is emitted on close (the server inits the subscriber with
        // `FmtSpan::CLOSE`), so each completed request is one line at the span's level.
        // Liveness/readiness probes get a debug-level span so they fall silent at the
        // info baseline while real requests stay at info.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request<Body>| {
                    let method = req.method();
                    let path = req.uri().path();
                    if is_probe(path) {
                        tracing::debug_span!(
                            "request", %method, %path,
                            status = field::Empty, latency_ms = field::Empty,
                        )
                    } else {
                        tracing::info_span!(
                            "request", %method, %path,
                            status = field::Empty, latency_ms = field::Empty,
                        )
                    }
                })
                // The span-close line is the request record, so per-request events are off.
                .on_request(())
                .on_response(|res: &Response<Body>, latency: Duration, span: &Span| {
                    span.record("status", res.status().as_u16());
                    span.record("latency_ms", latency.as_millis() as u64);
                })
                // Classifier failures include 5xx (already logged at the error
                // boundary), so only the transport-error variant — a dropped
                // connection, a body error — is worth its own line here.
                .on_failure(
                    |err: ServerErrorsFailureClass, latency: Duration, _span: &Span| {
                        if let ServerErrorsFailureClass::Error(e) = err {
                            tracing::warn!(
                                error = %e,
                                latency_ms = latency.as_millis() as u64,
                                "request transport failure"
                            );
                        }
                    },
                ),
        )
}

/// Health/readiness probes that k8s polls constantly — logged at debug so they don't
/// drown the request log at the info baseline.
fn is_probe(path: &str) -> bool {
    matches!(path, "/api/v1/healthz" | "/api/v1/readyz")
}

/// The OpenAPI document for the API, derived from the same routes that are served.
///
/// The port generics only fix the router's state type; the document is identical
/// for any choice, so callers (the export example, tests) may pass throwaway port
/// types — the handlers are never invoked.
pub fn openapi<U, S, I, P, R, G>() -> OpenApi
where
    U: UserStorage + StatusBackend + Clone + 'static,
    S: SessionRepository + StatusBackend + Clone + 'static,
    I: IdGenerator + Clone + 'static,
    P: SSOProvider + Clone + 'static,
    R: ReportStorage + StatusBackend + Clone + 'static,
    G: SignatureStorage + StatusBackend + Clone + 'static,
{
    api_router::<U, S, I, P, R, G>().split_for_parts().1
}
