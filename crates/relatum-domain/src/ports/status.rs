//! Contract for returning a status for backends that deem it necessary.

use std::future::Future;

use crate::DomainError;

/// A backend that may have trouble connecting or may encounter general difficulties
/// in operation, like a database or any kind of microservice.
///
/// `Send + Sync` and a `Send` future: these ports are held inside the domain
/// services, which the API layer shares across threads behind axum state, so both
/// the implementors and the futures they return must cross thread boundaries.
pub trait StatusBackend: Send + Sync {
    fn get_status(&self) -> impl Future<Output = PortStatus> + Send;
}

/// Status to be returned from ports, indicating the status of their respective
/// backends.
pub enum PortStatus {
    /// Backend is running as expected
    Healthy,
    /// No connection was established **this session**.
    NotConnected,
    /// Connection was made, but things are in a suboptimal state (or not working at
    /// all).
    ///
    /// reason contains a human readable error message explaining what is wrong.
    Unhealthy { reason: String },
}

/// The error returned when trying to convert [`PortStatus::Healthy`] into a
/// [`DomainError`].
pub struct NotAnError;

impl TryFrom<PortStatus> for DomainError {
    type Error = NotAnError;

    fn try_from(value: PortStatus) -> Result<Self, Self::Error> {
        match value {
            PortStatus::Healthy => Err(NotAnError),
            PortStatus::Unhealthy { reason } => {
                Ok(Self::Backend(format!("Backend is unhealthy: {reason}")))
            }
            PortStatus::NotConnected => Ok(Self::Backend("Could not connect to backend".to_string())),
        }
    }
}
