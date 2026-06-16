//! Service metadata and health-probe use-cases.
//!
//! [`MetaService`] reports "what is this service and can it serve". `health` and
//! `readiness` mirror Kubernetes liveness/readiness probes: `health` reports
//! whether the service has started and can accept requests, `readiness` whether it
//! can actually process them.
//!
//! It is generic over the stateful stores it probes — user (`U`), session (`S`) and
//! report (`R`) storage — so `readiness` can consult their [`StatusBackend`] and
//! report a real failure when a backend is unreachable. The SSO provider and the
//! directory are deliberately not probed: their "disabled"/unconfigured forms
//! legitimately report not-connected, which is not a readiness failure.

use crate::errors::DomainError;
use crate::models::meta::ServiceInfo;
use crate::ports::status::{NotAnError, PortStatus, StatusBackend};

/// Reports metadata and health for the running service.
#[derive(Debug, Clone)]
pub struct MetaService<U, S, R> {
    info: ServiceInfo,
    users: U,
    sessions: S,
    reports: R,
}

impl<U, S, R> MetaService<U, S, R> {
    /// Build the service from its name and semantic version, plus the stores whose
    /// health [`readiness`](Self::readiness) reports.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        users: U,
        sessions: S,
        reports: R,
    ) -> Self {
        Self {
            info: ServiceInfo {
                name: name.into(),
                version: version.into(),
            },
            users,
            sessions,
            reports,
        }
    }

    /// Return basic metadata about the running service.
    pub async fn info(&self) -> Result<ServiceInfo, DomainError> {
        Ok(self.info.clone())
    }

    /// Liveness check: the service has started and can accept requests.
    ///
    /// Independent of backend reachability — a transient store outage should not
    /// fail liveness (and, under Kubernetes, restart the pod).
    pub async fn health(&self) -> Result<(), DomainError> {
        Ok(())
    }
}

impl<U, S, R> MetaService<U, S, R>
where
    U: StatusBackend,
    S: StatusBackend,
    R: StatusBackend,
{
    /// Readiness check: the service can process incoming requests.
    ///
    /// Probes each backing store in turn and reports the first that is unhealthy or
    /// unreachable, so the service reports not-ready while a backend is down.
    pub async fn readiness(&self) -> Result<(), DomainError> {
        check(self.users.get_status().await)?;
        check(self.sessions.get_status().await)?;
        check(self.reports.get_status().await)?;
        Ok(())
    }
}

/// Turn a probed [`PortStatus`] into a readiness result: healthy passes, anything
/// else is surfaced as the corresponding [`DomainError`].
fn check(status: PortStatus) -> Result<(), DomainError> {
    match DomainError::try_from(status) {
        Ok(err) => Err(err),
        Err(NotAnError) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{InMemoryReports, InMemorySessions, InMemoryUsers, block_on};

    /// A backend whose status is fixed at construction, for exercising the failure
    /// paths the in-memory doubles never take.
    #[derive(Clone)]
    struct FixedStatus(fn() -> PortStatus);

    impl StatusBackend for FixedStatus {
        async fn get_status(&self) -> PortStatus {
            (self.0)()
        }
    }

    fn healthy() -> MetaService<InMemoryUsers, InMemorySessions, InMemoryReports> {
        MetaService::new(
            "relatum",
            "0.0.0",
            InMemoryUsers::default(),
            InMemorySessions::default(),
            InMemoryReports::default(),
        )
    }

    #[test]
    fn readiness_ok_when_all_backends_healthy() {
        assert!(block_on(healthy().readiness()).is_ok());
    }

    #[test]
    fn readiness_fails_when_a_backend_is_unhealthy() {
        let meta = MetaService::new(
            "relatum",
            "0.0.0",
            InMemoryUsers::default(),
            FixedStatus(|| PortStatus::Unhealthy {
                reason: "boom".to_string(),
            }),
            InMemoryReports::default(),
        );
        assert!(matches!(
            block_on(meta.readiness()),
            Err(DomainError::Backend(_))
        ));
    }

    #[test]
    fn readiness_fails_when_a_backend_is_not_connected() {
        let meta = MetaService::new(
            "relatum",
            "0.0.0",
            InMemoryUsers::default(),
            InMemorySessions::default(),
            FixedStatus(|| PortStatus::NotConnected),
        );
        assert!(matches!(
            block_on(meta.readiness()),
            Err(DomainError::Backend(_))
        ));
    }
}
