//! Service-metadata DTOs.

use relatum_domain::models::meta::ServiceInfo;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Basic metadata about the running API. Returned by the info endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiInfo {
    /// Service name, e.g. `"relatum"`.
    #[schema(example = "relatum")]
    pub name: String,
    /// Semantic version of the service.
    #[schema(example = "0.1.0")]
    pub version: String,
}

impl From<ServiceInfo> for ApiInfo {
    fn from(info: ServiceInfo) -> Self {
        ApiInfo {
            name: info.name,
            version: info.version,
        }
    }
}
