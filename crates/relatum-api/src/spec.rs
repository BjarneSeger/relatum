//! Static access to the OpenAPI document, independent of any running backend.
//!
//! The full spec — paths *and* schemas — is assembled from the live routes by
//! [`crate::routes::openapi`], which is generic over the five outbound ports only to
//! fix the router's state type; the document itself is identical for any choice. So
//! to produce it without a backend we plug in [`Dummy`], a zero-sized stand-in that
//! implements every port with `unimplemented!()` methods that are never called.
//!
//! This is the single source consumed by both `examples/export-openapi.rs` and any
//! out-of-process client generator (e.g. `relatum-client`'s build script), so the
//! generated client cannot drift from the served routes.

use jiff::Timestamp;
use relatum_domain::errors::DomainError;
use relatum_domain::models::auth::SessionToken;
use relatum_domain::models::ids::{DepartmentId, ReportId, UserId};
use relatum_domain::models::report::Report;
use relatum_domain::models::signature::{Signature, StoredSignature};
use relatum_domain::models::users::User;
use relatum_domain::ports::ids::IdGenerator;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::session::SessionRepository;
use relatum_domain::ports::signaturestorage::SignatureStorage;
use relatum_domain::ports::sso_connector::{SSOProvider, SsoIdentity};
use relatum_domain::ports::status::{PortStatus, StatusBackend};
use relatum_domain::ports::userstorage::UserStorage;
use utoipa::openapi::OpenApi;

/// A stand-in for every outbound port. It only fixes the router's state type so the
/// spec can be generated; its methods are never invoked.
#[derive(Clone)]
struct Dummy;

impl StatusBackend for Dummy {
    async fn get_status(&self) -> PortStatus {
        unimplemented!()
    }
}
impl UserStorage for Dummy {
    async fn store(&self, _: User) -> Result<(), DomainError> {
        unimplemented!()
    }
    async fn lookup(&self, _: &UserId) -> Result<Option<User>, DomainError> {
        unimplemented!()
    }
    async fn remove(&self, _: &UserId) -> Result<User, DomainError> {
        unimplemented!()
    }
    async fn list_all(&self) -> Result<Vec<User>, DomainError> {
        unimplemented!()
    }
}
impl SessionRepository for Dummy {
    async fn store(&self, _: &SessionToken) -> Result<(), DomainError> {
        unimplemented!()
    }
    async fn lookup(&self, _: &str) -> Result<Option<SessionToken>, DomainError> {
        unimplemented!()
    }
    async fn revoke(&self, _: &str) -> Result<(), DomainError> {
        unimplemented!()
    }
}
impl ReportStorage for Dummy {
    async fn store(&self, _: &Report) -> Result<(), DomainError> {
        unimplemented!()
    }
    async fn lookup(&self, _: &ReportId) -> Result<Option<Report>, DomainError> {
        unimplemented!()
    }
    async fn remove(&self, _: &ReportId) -> Result<Report, DomainError> {
        unimplemented!()
    }
    async fn list_by_author(&self, _: &UserId) -> Result<Vec<Report>, DomainError> {
        unimplemented!()
    }
    async fn list_by_department(&self, _: &DepartmentId) -> Result<Vec<Report>, DomainError> {
        unimplemented!()
    }
    async fn list_all(&self) -> Result<Vec<Report>, DomainError> {
        unimplemented!()
    }
}
impl IdGenerator for Dummy {
    fn report_id(&self) -> ReportId {
        unimplemented!()
    }
    fn session_token(&self) -> String {
        unimplemented!()
    }
}
impl SSOProvider for Dummy {
    async fn check_token(&self, _: &str) -> Result<Option<SsoIdentity>, DomainError> {
        unimplemented!()
    }
}
impl SignatureStorage for Dummy {
    async fn set(&self, _: &UserId, _: Signature, _: Timestamp) -> Result<(), DomainError> {
        unimplemented!()
    }
    async fn get(&self, _: &UserId) -> Result<Option<StoredSignature>, DomainError> {
        unimplemented!()
    }
}

/// The full OpenAPI document for the API, derived from the live routes.
pub fn openapi_doc() -> OpenApi {
    crate::routes::openapi::<Dummy, Dummy, Dummy, Dummy, Dummy, Dummy>()
}

/// The full OpenAPI document, serialized as pretty-printed JSON.
///
/// Convenience over [`openapi_doc`] for tooling that consumes the spec as a file
/// (the export example, the `relatum-client` build script).
pub fn openapi_json() -> Result<String, serde_json::Error> {
    openapi_doc().to_pretty_json()
}
