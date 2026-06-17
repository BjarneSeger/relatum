//! Shared application state injected into handlers.
//!
//! [`AppState`] bundles the domain services the HTTP layer calls. It holds the
//! **real** services by value, generic only over the five outbound ports they
//! share (`U`ser storage, `S`ession repository, `I`d generator, sso `P`rovider,
//! `R`eport storage). The ports promise `Send + Sync` futures, so the services —
//! and therefore this state — are `Clone + Send + Sync` and can sit behind axum's
//! shared state.
//!
//! Authentication is SSO-only, so there is no password hasher. User provisioning
//! happens out of band via the directory sync (run by the server), so the only
//! user-mutation the API exposes is department assignment through
//! [`UserAdmin`](relatum_domain::services::admin::UserAdmin).
//!
//! A server binary constructs it by wiring `relatum-infra` adapters into the
//! services; the integration tests construct it from in-crate fakes. Either way the
//! same [`router`](crate::routes::router) consumes it.

use relatum_domain::services::admin::UserAdmin;
use relatum_domain::services::auth::Authenticator;
use relatum_domain::services::meta::MetaService;
use relatum_domain::services::report::ReportService;
use relatum_domain::services::signature::SignatureService;

/// The services every handler can reach.
///
/// `auth`/`reports`/`admin` share the user-storage (`U`) and id-generation (`I`)
/// ports — there is one user store and one id source per instance. `G` is the
/// si`G`nature storage port: `reports` consults it to require a signature before a
/// submit/sign, and `signatures` is the self-service set/get use-case behind it.
#[derive(Debug, Clone)]
pub struct AppState<U, S, I, P, R, G> {
    /// Authentication use-cases (login / logout / refresh / authenticate).
    pub auth: Authenticator<U, S, I, P>,
    /// Service metadata and health probes.
    pub meta: MetaService<U, S, R>,
    /// The report submit → sign workflow.
    pub reports: ReportService<R, U, I, G>,
    /// Manual user administration (department assignment).
    pub admin: UserAdmin<U>,
    /// Self-service signature registration (set / get the caller's own).
    pub signatures: SignatureService<G>,
}

impl<U, S, I, P, R, G> AppState<U, S, I, P, R, G> {
    /// Bundle the concrete service implementations into one state value.
    pub fn new(
        auth: Authenticator<U, S, I, P>,
        meta: MetaService<U, S, R>,
        reports: ReportService<R, U, I, G>,
        admin: UserAdmin<U>,
        signatures: SignatureService<G>,
    ) -> Self {
        Self {
            auth,
            meta,
            reports,
            admin,
            signatures,
        }
    }
}
