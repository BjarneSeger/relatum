//! Report-review use-cases.
//!
//! [`ReportService`] is the concrete struct that drives the trainee → signer
//! workflow. It is the counterpart to the pure state machine on
//! [`Report`](crate::models::report::Report): the entity owns *which* transitions
//! are legal, while this service owns the two things the entity deliberately does
//! not — **authorization** and the **clock**.
//!
//! Authorization follows the queue model: a report is drafted and submitted by its
//! trainee author into their department's queue; a **signer in that department**
//! may sign or reject it; an **instructor** may read any queue but never decides
//! (instructors are global and read-only). Violations surface as
//! [`DomainError::Forbidden`]. The service stamps `Timestamp::now()` onto each
//! transition.
//!
//! It consumes the [`ReportStorage`](crate::ports::reportstorage::ReportStorage)
//! and [`UserStorage`](crate::ports::userstorage::UserStorage) ports to load and
//! persist reports and to resolve the acting user's role, the
//! [`IdGenerator`](crate::ports::ids::IdGenerator) port to mint fresh report ids,
//! and the [`SignatureStorage`](crate::ports::signaturestorage::SignatureStorage)
//! port to require that the acting trainee (on submit) or signer (on sign) has a
//! signature on file — so no report can reach the queue or be signed off without
//! one to render onto its eventual PDF.

use crate::errors::DomainError;
use crate::models::ids::{ReportId, UserId};
use crate::models::report::{Report, ReviewStatus};
use crate::models::signature::StoredSignature;
use crate::models::users::{Role, User};
use crate::models::week::IsoWeek;
use crate::ports::ids::IdGenerator;
use crate::ports::reportstorage::ReportStorage;
use crate::ports::signaturestorage::SignatureStorage;
use crate::ports::userstorage::UserStorage;
use jiff::Timestamp;

/// Everything the outer layers need to render a report (e.g. as a PDF): the report
/// itself, the author's display name and signature, and — once signed — the signer's.
///
/// Assembled by [`ReportService::export_inputs`] behind the same visibility check as
/// [`ReportService::get`], so reading another user's name and signature here never
/// bypasses authorization and needs no separate "read someone else's signature" port.
#[derive(Debug, Clone)]
pub struct ReportForExport {
    /// The report being exported.
    pub report: Report,
    /// The author's directory display name.
    pub author_name: String,
    /// The author's registered signature, or `None` if they have none on file.
    pub author_signature: Option<StoredSignature>,
    /// The signer's details, present only once the report is [`Signed`](ReviewStatus::Signed).
    pub signer: Option<SignerForExport>,
}

/// The signer side of a [`ReportForExport`].
#[derive(Debug, Clone)]
pub struct SignerForExport {
    /// The signer's directory display name.
    pub name: String,
    /// The signer's registered signature, or `None` if they have none on file.
    pub signature: Option<StoredSignature>,
    /// When the report was signed.
    pub signed_at: Timestamp,
}

/// Drives the report submit → sign workflow.
#[derive(Debug, Clone)]
pub struct ReportService<R, U, I, S> {
    reports: R,
    users: U,
    ids: I,
    signatures: S,
}

impl<R, U, I, S> ReportService<R, U, I, S>
where
    R: ReportStorage,
    U: UserStorage,
    I: IdGenerator,
    S: SignatureStorage,
{
    /// Wire the service to its storage, id-generation, and signature ports.
    pub fn new(reports: R, users: U, ids: I, signatures: S) -> Self {
        Self {
            reports,
            users,
            ids,
            signatures,
        }
    }

    /// Start a new draft report for `author` covering `week`, returning its fresh
    /// id.
    ///
    /// Only a trainee may author a report; anyone else is rejected with
    /// [`DomainError::Forbidden`]. A trainee may file at most one report per ISO
    /// week — a second draft for a week they already have is
    /// [`DomainError::Conflict`]. The report is filed into the author's
    /// department, which scopes the signers who can later sign it.
    pub async fn create_draft(
        &self,
        author: &UserId,
        week: IsoWeek,
        content: String,
    ) -> Result<ReportId, DomainError> {
        let role = self.role_of(author).await?;
        let Role::Trainee { department } = role else {
            return Err(DomainError::Forbidden(
                "only a trainee may author a report".into(),
            ));
        };

        // One report per trainee per week. The author's reports are few, so a
        // scan of their list is cheaper than a dedicated port method.
        if self
            .reports
            .list_by_author(author)
            .await?
            .iter()
            .any(|r| *r.week() == week)
        {
            return Err(DomainError::Conflict(format!(
                "a report already exists for {week}"
            )));
        }

        let id = self.ids.report_id();
        let report = Report::new(id.clone(), author.clone(), department, week, content);
        self.reports.store(&report).await?;
        Ok(id)
    }

    /// Replace the markdown of a draft, submitted, or rejected report. Only the
    /// author may revise their own report.
    pub async fn revise(
        &self,
        actor: &UserId,
        id: &ReportId,
        content: String,
    ) -> Result<(), DomainError> {
        let mut report = self.load_authored_by(actor, id, "revise").await?;
        report.revise(content)?;
        self.reports.store(&report).await
    }

    /// Submit a report into its department's queue. Only the author may submit;
    /// there is no chosen reviewer — every signer in the department sees it. The
    /// author must have a signature on file (so the eventual PDF can carry it);
    /// otherwise this fails with [`DomainError::Precondition`].
    pub async fn submit(&self, actor: &UserId, id: &ReportId) -> Result<(), DomainError> {
        let mut report = self.load_authored_by(actor, id, "submit").await?;
        self.ensure_signature(actor).await?;
        report.submit(Timestamp::now())?;
        self.reports.store(&report).await
    }

    /// Sign a submitted report. Only a signer in the report's department may sign,
    /// and only once they have a signature on file (else
    /// [`DomainError::Precondition`]).
    pub async fn sign(&self, actor: &UserId, id: &ReportId) -> Result<(), DomainError> {
        let mut report = self.load(id).await?;
        self.ensure_signer_for(actor, &report).await?;
        self.ensure_signature(actor).await?;
        report.sign(actor.clone(), Timestamp::now())?;
        self.reports.store(&report).await
    }

    /// Reject a submitted report with a reason. Only a signer in the report's
    /// department may reject.
    pub async fn reject(
        &self,
        actor: &UserId,
        id: &ReportId,
        reason: String,
    ) -> Result<(), DomainError> {
        let mut report = self.load(id).await?;
        self.ensure_signer_for(actor, &report).await?;
        report.reject(reason, Timestamp::now())?;
        self.reports.store(&report).await
    }

    /// Fetch a single report. Visible to its author, to a signer in its department,
    /// or to any instructor (instructors have global read access).
    pub async fn get(&self, actor: &UserId, id: &ReportId) -> Result<Report, DomainError> {
        let report = self.load(id).await?;
        if report.author() == actor {
            return Ok(report);
        }
        match self.role_of(actor).await? {
            Role::Instructor { .. } => Ok(report),
            Role::Signer { department } if department == *report.department() => Ok(report),
            _ => Err(DomainError::Forbidden(
                "you may not view this report".into(),
            )),
        }
    }

    /// Gather everything needed to render `id` for `actor`: the report, the author's
    /// name and signature, and — once signed — the signer's. Visible to exactly the
    /// same callers as [`get`](Self::get) (author, a signer in the report's
    /// department, or any instructor); other users get [`DomainError::Forbidden`].
    ///
    /// The signature images are read here, server-side, behind that one check, so the
    /// outer layers never need an endpoint that exposes another user's signature.
    pub async fn export_inputs(
        &self,
        actor: &UserId,
        id: &ReportId,
    ) -> Result<ReportForExport, DomainError> {
        let report = self.get(actor, id).await?;
        let author = self.load_user(report.author()).await?;
        let author_signature = self.signatures.get(report.author()).await?;

        let signer = match report.status() {
            ReviewStatus::Signed { by, at } => {
                let user = self.load_user(by).await?;
                let signature = self.signatures.get(by).await?;
                Some(SignerForExport {
                    name: user.username().to_owned(),
                    signature,
                    signed_at: *at,
                })
            }
            _ => None,
        };

        Ok(ReportForExport {
            report,
            author_name: author.username().to_owned(),
            author_signature,
            signer,
        })
    }

    /// List the reports `actor` is involved in:
    /// - a **trainee** sees the reports they authored;
    /// - a **signer** sees their department's queue;
    /// - an **instructor** sees every report (global, read-only).
    pub async fn list_for(&self, actor: &UserId) -> Result<Vec<Report>, DomainError> {
        match self.role_of(actor).await? {
            Role::Trainee { .. } => self.reports.list_by_author(actor).await,
            Role::Signer { department } => self.reports.list_by_department(&department).await,
            Role::Instructor { .. } => self.reports.list_all().await,
        }
    }

    /// The acting user's effective role, or [`DomainError::Forbidden`] if they are
    /// inert (no department assigned).
    async fn role_of(&self, id: &UserId) -> Result<Role, DomainError> {
        let user = self.load_user(id).await?;
        user.role()
            .ok_or_else(|| DomainError::Forbidden("you have no department assigned".into()))
    }

    /// Assert `actor` is a signer in `report`'s department.
    async fn ensure_signer_for(&self, actor: &UserId, report: &Report) -> Result<(), DomainError> {
        match self.role_of(actor).await? {
            Role::Signer { department } if department == *report.department() => Ok(()),
            _ => Err(DomainError::Forbidden(
                "only a signer in this report's department may sign or reject it".into(),
            )),
        }
    }

    /// Assert `actor` has a signature on file. Required before a trainee may submit
    /// a report or a signer may sign one, so the eventual PDF always has a mark to
    /// render. Surfaces as [`DomainError::Precondition`] so the API can prompt the
    /// caller to register one.
    async fn ensure_signature(&self, actor: &UserId) -> Result<(), DomainError> {
        if self.signatures.get(actor).await?.is_some() {
            Ok(())
        } else {
            Err(DomainError::Precondition(
                "register a signature before submitting or signing a report".into(),
            ))
        }
    }

    /// Load a user by id, mapping a missing one to [`DomainError::NotFound`].
    async fn load_user(&self, id: &UserId) -> Result<User, DomainError> {
        self.users
            .lookup(id)
            .await?
            .ok_or_else(|| DomainError::NotFound(format!("user {}", id.as_str())))
    }

    /// Load a report by id, mapping a missing one to [`DomainError::NotFound`].
    async fn load(&self, id: &ReportId) -> Result<Report, DomainError> {
        self.reports
            .lookup(id)
            .await?
            .ok_or_else(|| DomainError::NotFound(format!("report {}", id.as_str())))
    }

    /// Load a report and assert `actor` is its author, for author-only actions.
    async fn load_authored_by(
        &self,
        actor: &UserId,
        id: &ReportId,
        action: &str,
    ) -> Result<Report, DomainError> {
        let report = self.load(id).await?;
        if report.author() != actor {
            return Err(DomainError::Forbidden(format!(
                "only the author may {action} this report"
            )));
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ids::DepartmentId;
    use crate::models::report::ReviewStatus;
    use crate::models::users::{DirectoryMarker, User};
    use crate::testing::{
        InMemoryReports, InMemorySignatures, InMemoryUsers, SeqIds, block_on, dev_signature,
    };

    type Reports = ReportService<InMemoryReports, InMemoryUsers, SeqIds, InMemorySignatures>;

    const INSTRUCTOR: &str = "ins";
    const SIGNER: &str = "sig";
    const TRAINEE: &str = "tr";
    const DEPT: &str = "blue";

    fn user(name: &str, marker: DirectoryMarker, department: Option<&str>) -> User {
        User::new(
            UserId::new(name),
            name,
            marker,
            department.map(DepartmentId::new),
        )
    }

    /// A service seeded with an instructor, a signer, and a trainee, all in `blue`.
    /// The trainee and signer have registered signatures, so the submit/sign gate is
    /// satisfied in the workflow tests; the gate tests use fresh users without one.
    fn service() -> Reports {
        let users = InMemoryUsers::default();
        block_on(users.store(user(INSTRUCTOR, DirectoryMarker::Instructor, Some(DEPT)))).unwrap();
        block_on(users.store(user(SIGNER, DirectoryMarker::Regular, Some(DEPT)))).unwrap();
        block_on(users.store(user(TRAINEE, DirectoryMarker::Trainee, Some(DEPT)))).unwrap();

        let signatures = InMemorySignatures::default();
        block_on(signatures.set(&id(TRAINEE), dev_signature(), seed_at())).unwrap();
        block_on(signatures.set(&id(SIGNER), dev_signature(), seed_at())).unwrap();

        ReportService::new(
            InMemoryReports::default(),
            users,
            SeqIds::default(),
            signatures,
        )
    }

    fn id(name: &str) -> UserId {
        UserId::new(name)
    }

    /// A fixed instant used when seeding signatures in tests.
    fn seed_at() -> Timestamp {
        "2026-06-10T12:00:00Z".parse().unwrap()
    }

    /// A valid ISO week to file test reports under.
    fn wk(week: i8) -> IsoWeek {
        IsoWeek::new(2026, week).unwrap()
    }

    fn submitted_report(svc: &Reports) -> ReportId {
        let report_id = block_on(svc.create_draft(&id(TRAINEE), wk(24), "# done".into())).unwrap();
        block_on(svc.submit(&id(TRAINEE), &report_id)).unwrap();
        report_id
    }

    #[test]
    fn trainee_creates_a_draft_in_their_department() {
        let svc = service();
        let report_id =
            block_on(svc.create_draft(&id(TRAINEE), wk(24), "# week 1".into())).unwrap();

        let report = block_on(svc.reports.lookup(&report_id)).unwrap().unwrap();
        assert_eq!(report.author().as_str(), TRAINEE);
        assert_eq!(report.department().as_str(), DEPT);
        assert!(matches!(report.status(), ReviewStatus::Draft));
    }

    #[test]
    fn at_most_one_report_per_trainee_per_week() {
        let svc = service();
        block_on(svc.create_draft(&id(TRAINEE), wk(24), "# first".into())).unwrap();

        // A second report for the same week is a conflict...
        let err = block_on(svc.create_draft(&id(TRAINEE), wk(24), "# dup".into())).unwrap_err();
        assert!(matches!(err, DomainError::Conflict(_)));

        // ...but a different week is fine.
        assert!(block_on(svc.create_draft(&id(TRAINEE), wk(25), "# next".into())).is_ok());
    }

    #[test]
    fn only_a_trainee_may_author_a_report() {
        let svc = service();
        for non_trainee in [INSTRUCTOR, SIGNER] {
            let err = block_on(svc.create_draft(&id(non_trainee), wk(24), "x".into())).unwrap_err();
            assert!(matches!(err, DomainError::Forbidden(_)));
        }
    }

    #[test]
    fn a_user_without_a_department_cannot_author() {
        let svc = service();
        block_on(
            svc.users
                .store(user("inert", DirectoryMarker::Trainee, None)),
        )
        .unwrap();
        let err = block_on(svc.create_draft(&id("inert"), wk(24), "x".into())).unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[test]
    fn create_draft_for_an_unknown_user_is_not_found() {
        let err = block_on(service().create_draft(&id("ghost"), wk(24), "x".into())).unwrap_err();
        assert!(matches!(err, DomainError::NotFound(_)));
    }

    #[test]
    fn only_the_author_may_revise_and_submit() {
        let svc = service();
        let report_id = block_on(svc.create_draft(&id(TRAINEE), wk(24), "x".into())).unwrap();

        let revise_err = block_on(svc.revise(&id(SIGNER), &report_id, "y".into())).unwrap_err();
        assert!(matches!(revise_err, DomainError::Forbidden(_)));

        let submit_err = block_on(svc.submit(&id(SIGNER), &report_id)).unwrap_err();
        assert!(matches!(submit_err, DomainError::Forbidden(_)));
    }

    #[test]
    fn a_signer_in_the_department_can_sign() {
        let svc = service();
        let report_id = submitted_report(&svc);

        block_on(svc.sign(&id(SIGNER), &report_id)).unwrap();
        match block_on(svc.get(&id(SIGNER), &report_id)).unwrap().status() {
            ReviewStatus::Signed { by, .. } => assert_eq!(by.as_str(), SIGNER),
            other => panic!("expected Signed, got {other:?}"),
        }
    }

    #[test]
    fn a_signer_from_another_department_cannot_sign() {
        let svc = service();
        block_on(
            svc.users
                .store(user("other-sig", DirectoryMarker::Regular, Some("red"))),
        )
        .unwrap();
        let report_id = submitted_report(&svc);

        let err = block_on(svc.sign(&id("other-sig"), &report_id)).unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[test]
    fn a_trainee_without_a_signature_cannot_submit() {
        let svc = service();
        // A fresh trainee in the department, no signature on file.
        block_on(
            svc.users
                .store(user("tr2", DirectoryMarker::Trainee, Some(DEPT))),
        )
        .unwrap();
        let report_id = block_on(svc.create_draft(&id("tr2"), wk(24), "# done".into())).unwrap();

        let err = block_on(svc.submit(&id("tr2"), &report_id)).unwrap_err();
        assert!(matches!(err, DomainError::Precondition(_)));
        // The failed submit must not advance the report.
        assert!(matches!(
            block_on(svc.reports.lookup(&report_id))
                .unwrap()
                .unwrap()
                .status(),
            ReviewStatus::Draft
        ));

        // Once they register a signature, the same submit succeeds.
        block_on(svc.signatures.set(&id("tr2"), dev_signature(), seed_at())).unwrap();
        block_on(svc.submit(&id("tr2"), &report_id)).unwrap();
        assert!(matches!(
            block_on(svc.reports.lookup(&report_id))
                .unwrap()
                .unwrap()
                .status(),
            ReviewStatus::Submitted { .. }
        ));
    }

    #[test]
    fn a_signer_without_a_signature_cannot_sign() {
        let svc = service();
        // A second signer in the department, no signature on file.
        block_on(
            svc.users
                .store(user("sig2", DirectoryMarker::Regular, Some(DEPT))),
        )
        .unwrap();
        let report_id = submitted_report(&svc);

        let err = block_on(svc.sign(&id("sig2"), &report_id)).unwrap_err();
        assert!(matches!(err, DomainError::Precondition(_)));
        // Still awaiting a signature — the failed sign must not advance it.
        assert!(matches!(
            block_on(svc.reports.lookup(&report_id))
                .unwrap()
                .unwrap()
                .status(),
            ReviewStatus::Submitted { .. }
        ));

        // After registering, the signer can sign.
        block_on(svc.signatures.set(&id("sig2"), dev_signature(), seed_at())).unwrap();
        block_on(svc.sign(&id("sig2"), &report_id)).unwrap();
        match block_on(svc.get(&id("sig2"), &report_id)).unwrap().status() {
            ReviewStatus::Signed { by, .. } => assert_eq!(by.as_str(), "sig2"),
            other => panic!("expected Signed, got {other:?}"),
        }
    }

    #[test]
    fn an_instructor_can_read_but_not_sign() {
        let svc = service();
        let report_id = submitted_report(&svc);

        // Read access: an instructor sees any report.
        block_on(svc.get(&id(INSTRUCTOR), &report_id)).unwrap();

        // But cannot sign or reject.
        let sign_err = block_on(svc.sign(&id(INSTRUCTOR), &report_id)).unwrap_err();
        assert!(matches!(sign_err, DomainError::Forbidden(_)));
        let reject_err =
            block_on(svc.reject(&id(INSTRUCTOR), &report_id, "no".into())).unwrap_err();
        assert!(matches!(reject_err, DomainError::Forbidden(_)));
    }

    #[test]
    fn reject_lets_the_author_revise_and_resubmit() {
        let svc = service();
        let report_id = submitted_report(&svc);
        block_on(svc.reject(&id(SIGNER), &report_id, "needs detail".into())).unwrap();

        block_on(svc.revise(&id(TRAINEE), &report_id, "# better".into())).unwrap();
        block_on(svc.submit(&id(TRAINEE), &report_id)).unwrap();
        block_on(svc.sign(&id(SIGNER), &report_id)).unwrap();
        assert!(matches!(
            block_on(svc.get(&id(SIGNER), &report_id)).unwrap().status(),
            ReviewStatus::Signed { .. }
        ));
    }

    #[test]
    fn export_inputs_gathers_author_and_signer() {
        let svc = service();
        let report_id = submitted_report(&svc);
        block_on(svc.sign(&id(SIGNER), &report_id)).unwrap();

        let ex = block_on(svc.export_inputs(&id(TRAINEE), &report_id)).unwrap();
        // In the fixtures a user's display name equals their id.
        assert_eq!(ex.author_name, TRAINEE);
        assert!(
            ex.author_signature.is_some(),
            "author has a signature on file"
        );
        let signer = ex.signer.expect("a signed report carries a signer");
        assert_eq!(signer.name, SIGNER);
        assert!(signer.signature.is_some(), "signer has a signature on file");
    }

    #[test]
    fn export_inputs_for_an_unsigned_report_has_no_signer() {
        let svc = service();
        let report_id = submitted_report(&svc); // submitted, not yet signed
        let ex = block_on(svc.export_inputs(&id(TRAINEE), &report_id)).unwrap();
        assert!(ex.signer.is_none());
        assert!(ex.author_signature.is_some());
    }

    #[test]
    fn export_inputs_respects_report_visibility() {
        let svc = service();
        block_on(
            svc.users
                .store(user("outsider", DirectoryMarker::Regular, Some("red"))),
        )
        .unwrap();
        let report_id = block_on(svc.create_draft(&id(TRAINEE), wk(24), "x".into())).unwrap();

        let err = block_on(svc.export_inputs(&id("outsider"), &report_id)).unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[test]
    fn get_is_forbidden_for_an_unrelated_user() {
        let svc = service();
        block_on(
            svc.users
                .store(user("outsider", DirectoryMarker::Regular, Some("red"))),
        )
        .unwrap();
        let report_id = block_on(svc.create_draft(&id(TRAINEE), wk(24), "x".into())).unwrap();

        let err = block_on(svc.get(&id("outsider"), &report_id)).unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)));
    }

    #[test]
    fn list_for_branches_on_role() {
        let svc = service();
        let report_id = submitted_report(&svc);

        // Trainee: their own authored reports.
        assert_eq!(block_on(svc.list_for(&id(TRAINEE))).unwrap().len(), 1);

        // Signer: the department queue.
        let queue = block_on(svc.list_for(&id(SIGNER))).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id().as_str(), report_id.as_str());

        // Instructor: every report, globally.
        assert_eq!(block_on(svc.list_for(&id(INSTRUCTOR))).unwrap().len(), 1);
    }
}
