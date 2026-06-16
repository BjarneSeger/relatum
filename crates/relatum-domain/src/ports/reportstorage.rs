//! Persistence contract for reports.
//!
//! Implemented in `relatum-infra` (e.g. backed by SQLite). The report service
//! depends on this port to load and persist reports without knowing the storage
//! details. Shaped to mirror [`UserStorage`](crate::ports::userstorage::UserStorage).
//!
//! The methods return `Send` futures so the report service can run behind the
//! API's shared axum state (`StatusBackend` already requires `Send + Sync`).

use std::future::Future;

use crate::DomainError;
use crate::models::ids::{DepartmentId, ReportId, UserId};
use crate::models::report::Report;
use crate::ports::status::StatusBackend;

pub trait ReportStorage: StatusBackend {
    /// Persist a report, inserting it or overwriting the existing one with the
    /// same id.
    fn store(&self, report: &Report) -> impl Future<Output = Result<(), DomainError>> + Send;

    /// Load a report by id, returning `None` if it does not exist.
    fn lookup(
        &self,
        id: &ReportId,
    ) -> impl Future<Output = Result<Option<Report>, DomainError>> + Send;

    /// Remove a report from storage, returning the removed value.
    fn remove(&self, id: &ReportId) -> impl Future<Output = Result<Report, DomainError>> + Send;

    /// All reports written by a given trainee.
    fn list_by_author(
        &self,
        author: &UserId,
    ) -> impl Future<Output = Result<Vec<Report>, DomainError>> + Send;

    /// All reports in a department's queue — i.e. authored by trainees in that
    /// department. Powers a signer's view of the reports they may sign.
    fn list_by_department(
        &self,
        department: &DepartmentId,
    ) -> impl Future<Output = Result<Vec<Report>, DomainError>> + Send;

    /// Every report in the instance. Powers an instructor's global, read-only view
    /// across all department queues.
    fn list_all(&self) -> impl Future<Output = Result<Vec<Report>, DomainError>> + Send;
}
