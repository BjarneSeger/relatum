//! In-memory [`ReportStorage`] backed by a `HashMap`, keyed by [`ReportId`].
//!
//! Self-contained: it needs no external service, so it is the natural default
//! for development, single-node deployments and tests. Cheap to clone — clones
//! share one `Arc<Mutex<…>>` — so the store can be handed to several handlers.
//! It is `Send + Sync`: the mutex guard is never held across an `.await`, so the
//! in-trait async futures stay `Send`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use relatum_domain::errors::DomainError;
use relatum_domain::models::ids::{DepartmentId, ReportId, UserId};
use relatum_domain::models::report::Report;
#[cfg(test)]
use relatum_domain::models::week::IsoWeek;
use relatum_domain::ports::reportstorage::ReportStorage;
use relatum_domain::ports::status::{PortStatus, StatusBackend};

/// In-memory [`ReportStorage`], keyed by [`ReportId`].
#[derive(Clone, Default)]
pub struct InMemoryReports {
    reports: Arc<Mutex<HashMap<String, Report>>>,
}

impl InMemoryReports {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ReportStorage for InMemoryReports {
    async fn store(&self, report: &Report) -> Result<(), DomainError> {
        self.reports
            .lock()
            .unwrap()
            .insert(report.id().as_str().to_owned(), report.clone());
        Ok(())
    }

    async fn lookup(&self, id: &ReportId) -> Result<Option<Report>, DomainError> {
        Ok(self.reports.lock().unwrap().get(id.as_str()).cloned())
    }

    async fn remove(&self, id: &ReportId) -> Result<Report, DomainError> {
        self.reports
            .lock()
            .unwrap()
            .remove(id.as_str())
            .ok_or_else(|| DomainError::NotFound(format!("report {}", id.as_str())))
    }

    async fn list_by_author(&self, author: &UserId) -> Result<Vec<Report>, DomainError> {
        Ok(self
            .reports
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.author() == author)
            .cloned()
            .collect())
    }

    async fn list_by_department(
        &self,
        department: &DepartmentId,
    ) -> Result<Vec<Report>, DomainError> {
        Ok(self
            .reports
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.department() == department)
            .cloned()
            .collect())
    }

    async fn list_all(&self) -> Result<Vec<Report>, DomainError> {
        Ok(self.reports.lock().unwrap().values().cloned().collect())
    }
}

impl StatusBackend for InMemoryReports {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draft report authored in `department`.
    fn report(id: &str, author: &str, department: &str) -> Report {
        Report::new(
            ReportId::new(id),
            UserId::new(author),
            DepartmentId::new(department),
            IsoWeek::new(2026, 24).unwrap(),
            "# Week 1\n\nDid things.".to_owned(),
        )
    }

    #[tokio::test]
    async fn stores_and_looks_up_a_report() {
        let store = InMemoryReports::new();
        store.store(&report("r1", "alice", "blue")).await.unwrap();

        let found = store.lookup(&ReportId::new("r1")).await.unwrap();
        assert_eq!(
            found.map(|r| r.id().as_str().to_owned()),
            Some("r1".to_owned())
        );
    }

    #[tokio::test]
    async fn unknown_report_is_none() {
        let store = InMemoryReports::new();
        assert!(
            store
                .lookup(&ReportId::new("nope"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn remove_returns_the_report_then_absent() {
        let store = InMemoryReports::new();
        store.store(&report("r1", "alice", "blue")).await.unwrap();

        let removed = store.remove(&ReportId::new("r1")).await.unwrap();
        assert_eq!(removed.id().as_str(), "r1");
        assert!(store.lookup(&ReportId::new("r1")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_absent_report_is_not_found() {
        let store = InMemoryReports::new();
        assert!(matches!(
            store.remove(&ReportId::new("ghost")).await,
            Err(DomainError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn lists_by_author_and_department() {
        let store = InMemoryReports::new();
        store.store(&report("r1", "alice", "blue")).await.unwrap();
        store.store(&report("r2", "alice", "blue")).await.unwrap();
        store.store(&report("r3", "dave", "red")).await.unwrap();

        let by_alice = store.list_by_author(&UserId::new("alice")).await.unwrap();
        assert_eq!(by_alice.len(), 2);
        assert!(by_alice.iter().all(|r| r.author().as_str() == "alice"));

        let in_blue = store
            .list_by_department(&DepartmentId::new("blue"))
            .await
            .unwrap();
        assert_eq!(in_blue.len(), 2);
        assert!(in_blue.iter().all(|r| r.department().as_str() == "blue"));

        assert_eq!(store.list_all().await.unwrap().len(), 3);
        assert!(
            store
                .list_by_author(&UserId::new("nobody"))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
