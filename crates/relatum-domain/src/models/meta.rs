//! Service-metadata value objects.

/// Basic metadata about the running service.
///
/// The API maps this to its `ApiInfo` DTO at the boundary.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    /// Service name, e.g. `"relatum"`.
    pub name: String,
    /// Semantic version of the service.
    pub version: String,
}
