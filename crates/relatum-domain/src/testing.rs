//! In-memory port doubles and a tiny executor for testing the services.
//!
//! This is the single home for the fakes the services are tested over. This
//! crate's own unit tests use them (they are compiled under `cfg(test)`), and
//! `relatum-api`'s HTTP tests use them too via the `testing` feature — so the
//! in-memory ports are maintained in exactly one place rather than copied per
//! crate.
//!
//! Each fake is `Clone` and `Arc`-backed where it holds state, so clones share one
//! backing store. That is what lets a double sit *by value* inside a service and
//! still be observed from the test (clone it before handing it over), and it lets
//! the services sit behind axum's shared, cloneable state.
//!
//! The domain stays dependency-pure even under test: rather than pull in an async
//! runtime, [`block_on`] drives a future to completion with the std no-op waker.
//! That is sound here because every fake resolves synchronously (it never returns
//! `Poll::Pending`), so the spin loop never actually spins.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::DomainError;
use crate::models::auth::SessionToken;
use crate::models::ids::{DepartmentId, ReportId, UserId};
use crate::models::report::Report;
use crate::models::users::{DirectoryMarker, User};
use crate::ports::directory::{DirectoryEntry, DirectorySource};
use crate::ports::ids::IdGenerator;
use crate::ports::reportstorage::ReportStorage;
use crate::ports::session::SessionRepository;
use crate::ports::sso_connector::{SSOProvider, SsoIdentity};
use crate::ports::status::{PortStatus, StatusBackend};
use crate::ports::userstorage::UserStorage;

/// Drive a future to completion on the current thread. All fakes here are
/// always-ready, so a single poll suffices.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

/// The canonical dev/test users: `(id, marker, department)`.
///
/// The single source of truth shared by the API's HTTP-test fixture and the
/// server's `mock` dev backend, so the two can never drift. Three users sit in
/// department `blue` (instructor `ins`, trainee `tr`, signer `sig`), one signer
/// `out` sits in the unrelated department `red`, and `newbie` has no department
/// (inert). Each user's SSO token is [`dev_token`] of its id.
pub fn dev_user_catalogue() -> Vec<(UserId, DirectoryMarker, Option<DepartmentId>)> {
    [
        ("ins", DirectoryMarker::Instructor, Some("blue")),
        ("tr", DirectoryMarker::Trainee, Some("blue")),
        ("sig", DirectoryMarker::Regular, Some("blue")),
        ("out", DirectoryMarker::Regular, Some("red")),
        ("newbie", DirectoryMarker::Regular, None),
    ]
    .into_iter()
    .map(|(id, marker, dept)| (UserId::new(id), marker, dept.map(DepartmentId::new)))
    .collect()
}

/// The SSO token the mock provider attests for the user with id `id`.
pub fn dev_token(id: &str) -> String {
    format!("tok-{id}")
}

/// Deterministic id generator: monotonically numbered ids and tokens.
#[derive(Clone, Default)]
pub struct SeqIds {
    reports: Arc<Mutex<u64>>,
    tokens: Arc<Mutex<u64>>,
}

impl IdGenerator for SeqIds {
    fn report_id(&self) -> ReportId {
        let mut n = self.reports.lock().unwrap();
        *n += 1;
        ReportId::new(format!("report-{n}"))
    }

    fn session_token(&self) -> String {
        let mut n = self.tokens.lock().unwrap();
        *n += 1;
        format!("token-{n}")
    }
}

/// In-memory [`UserStorage`], keyed by [`UserId`].
#[derive(Clone, Default)]
pub struct InMemoryUsers {
    users: Arc<Mutex<HashMap<String, User>>>,
}

impl UserStorage for InMemoryUsers {
    async fn store(&self, user: User) -> Result<(), DomainError> {
        self.users
            .lock()
            .unwrap()
            .insert(user.id().as_str().to_owned(), user);
        Ok(())
    }

    async fn lookup(&self, user: &UserId) -> Result<Option<User>, DomainError> {
        Ok(self.users.lock().unwrap().get(user.as_str()).cloned())
    }

    async fn remove(&self, user: &UserId) -> Result<User, DomainError> {
        self.users
            .lock()
            .unwrap()
            .remove(user.as_str())
            .ok_or_else(|| DomainError::NotFound(format!("user {}", user.as_str())))
    }

    async fn list_all(&self) -> Result<Vec<User>, DomainError> {
        Ok(self.users.lock().unwrap().values().cloned().collect())
    }
}

impl StatusBackend for InMemoryUsers {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

/// In-memory [`SessionRepository`], keyed by token value.
#[derive(Clone, Default)]
pub struct InMemorySessions {
    tokens: Arc<Mutex<HashMap<String, SessionToken>>>,
}

impl SessionRepository for InMemorySessions {
    async fn store(&self, token: &SessionToken) -> Result<(), DomainError> {
        self.tokens
            .lock()
            .unwrap()
            .insert(token.value.clone(), token.clone());
        Ok(())
    }

    async fn lookup(&self, value: &str) -> Result<Option<SessionToken>, DomainError> {
        Ok(self.tokens.lock().unwrap().get(value).cloned())
    }

    async fn revoke(&self, value: &str) -> Result<(), DomainError> {
        self.tokens.lock().unwrap().remove(value);
        Ok(())
    }
}

impl StatusBackend for InMemorySessions {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

/// Fake [`SSOProvider`]: only the tokens it has been told about are valid, each
/// mapping to the [`SsoIdentity`] it attests. An unregistered token is treated as
/// invalid (`None`), never as a backend error.
///
/// It also models the [single-use handoff](SSOProvider::stash_handoff) store so the
/// browser-flow exchange can be exercised without a live IdP: `stash_handoff` keys an
/// access token under a fresh code, and `redeem_handoff` removes and returns it.
#[derive(Clone, Default)]
pub struct MockSSO {
    tokens: Arc<Mutex<HashMap<String, SsoIdentity>>>,
    handoffs: Arc<Mutex<HashMap<String, String>>>,
    next_handoff: Arc<Mutex<u64>>,
}

impl MockSSO {
    /// Register `token` as a valid SSO token attesting `identity`.
    pub fn register(&self, token: &str, identity: SsoIdentity) {
        self.tokens
            .lock()
            .unwrap()
            .insert(token.to_owned(), identity);
    }
}

impl SSOProvider for MockSSO {
    async fn check_token(&self, token: &str) -> Result<Option<SsoIdentity>, DomainError> {
        Ok(self.tokens.lock().unwrap().get(token).cloned())
    }

    async fn stash_handoff(&self, access_token: String) -> Result<String, DomainError> {
        let mut n = self.next_handoff.lock().unwrap();
        *n += 1;
        let code = format!("handoff-{n}");
        self.handoffs.lock().unwrap().insert(code.clone(), access_token);
        Ok(code)
    }

    async fn redeem_handoff(&self, code: &str) -> Result<String, DomainError> {
        self.handoffs
            .lock()
            .unwrap()
            .remove(code)
            .ok_or_else(|| DomainError::Unauthorized("invalid or expired SSO handoff".into()))
    }
}

impl StatusBackend for MockSSO {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}

/// In-memory [`ReportStorage`], keyed by [`ReportId`].
#[derive(Clone, Default)]
pub struct InMemoryReports {
    reports: Arc<Mutex<HashMap<String, Report>>>,
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

/// In-memory [`DirectorySource`]: returns whatever entries it was last told to
/// hold, so a test can drive a sync against a controlled directory snapshot.
#[derive(Clone, Default)]
pub struct InMemoryDirectory {
    entries: Arc<Mutex<Vec<DirectoryEntry>>>,
}

impl InMemoryDirectory {
    /// Replace the directory's contents with `entries`.
    pub fn set(&self, entries: impl IntoIterator<Item = DirectoryEntry>) {
        *self.entries.lock().unwrap() = entries.into_iter().collect();
    }
}

impl DirectorySource for InMemoryDirectory {
    async fn list_entries(&self) -> Result<Vec<DirectoryEntry>, DomainError> {
        Ok(self.entries.lock().unwrap().clone())
    }
}

impl StatusBackend for InMemoryDirectory {
    async fn get_status(&self) -> PortStatus {
        PortStatus::Healthy
    }
}
