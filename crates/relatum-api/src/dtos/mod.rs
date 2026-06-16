//! Data-transfer objects: the JSON-serializable shapes that cross the API boundary.
//!
//! DTOs are **separate from the domain models** in `relatum-domain` on purpose:
//! the wire format (and its `serde`/`utoipa` derives) can evolve without touching
//! business types, and the domain stays free of serialization concerns. Each DTO
//! provides a conversion from its domain counterpart, applied at the handler
//! boundary.
//!
//! Convention: timestamps and dates are RFC3339 `String`s, which keeps the
//! OpenAPI schema simple and avoids a date-library/`ToSchema` integration.
//!
//! Remember to register every new DTO in [`crate::openapi::ApiDoc`].

pub mod auth;
pub mod meta;
pub mod report;
pub mod user;

pub use auth::{AuthSuccess, LoginRequest, SsoExchangeRequest, SsoInfo, Token};
pub use meta::ApiInfo;
pub use report::{
    CreateReportRequest, CreatedReport, ReportView, ReviewDecisionDto, ReviewRequest,
    ReviewStatusDto, ReviseReportRequest,
};
pub use user::{AssignDepartmentRequest, MarkerDto, MeView, RoleDto, UserSummary};
