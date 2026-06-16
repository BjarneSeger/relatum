//! Reports written by trainees and signed by their department's signers.
//!
//! A [`Report`] is the aggregate at the heart of this crate. A trainee drafts one
//! as markdown, then submits it into their department's queue — there is no chosen
//! reviewer; the report is visible to every signer in that department (and, read
//! only, to every instructor). A signer either signs it or rejects it with a
//! reason; a rejected report can be revised and resubmitted, looping until signed.
//!
//! The methods here own the *state* invariants — they are the single place that
//! decides which transition is legal from which status. They are deliberately
//! **pure**: no I/O, no clock, no notion of *who* is calling. Authorization
//! (only the author submits, only a signer in the report's department signs or
//! rejects) and the wall clock live one layer out, in
//! [`ReportService`](crate::services::report::ReportService), so this logic stays
//! trivially unit-testable.

use crate::DomainError;
use crate::models::ids::{DepartmentId, ReportId, UserId};
use crate::models::week::IsoWeek;
use jiff::Timestamp;

/// A single report and where it is in the review workflow.
#[derive(Debug, Clone)]
pub struct Report {
    id: ReportId,
    /// The trainee who wrote the report.
    author: UserId,
    /// The department whose queue this report belongs to — captured from the
    /// author when the draft is created. Signers in this department may sign it.
    department: DepartmentId,
    /// The ISO week this report covers. A trainee files at most one report per
    /// week (see [`ReportService`](crate::services::report::ReportService)).
    week: IsoWeek,
    /// The report body, as markdown.
    content: String,
    status: ReviewStatus,
}

/// Where a [`Report`] sits in the submit → sign cycle.
///
/// ```text
///   ┌──────────── revise ┐
///   ▼                    │
/// Draft ── submit ─► Submitted ── sign ─► Signed
///   ▲                    │
///   │                    └── reject(reason) ─► Rejected
///   └──────────── revise ◄────────────────────────┘
/// ```
#[derive(Debug, Clone)]
pub enum ReviewStatus {
    /// Being written or revised by the trainee; not yet in the queue.
    Draft,
    /// In the department queue, awaiting a signer's verdict.
    Submitted { at: Timestamp },
    /// A signer signed the report; `by` records who put their signature on it.
    Signed { at: Timestamp, by: UserId },
    /// A signer rejected the report; `reason` explains why so the trainee can
    /// address it and resubmit.
    Rejected { at: Timestamp, reason: String },
}

/// A signer's verdict on a submitted report.
///
/// Couples the two outcomes of a single decision so the service layer can
/// authorize and apply them in one place.
#[derive(Debug, Clone)]
pub enum ReviewDecision {
    Sign,
    Reject { reason: String },
}

impl Report {
    /// Start a fresh report in [`ReviewStatus::Draft`], belonging to `department`
    /// (the author's department at the time of drafting) and covering `week`.
    pub fn new(
        id: ReportId,
        author: UserId,
        department: DepartmentId,
        week: IsoWeek,
        content: String,
    ) -> Self {
        Self {
            id,
            author,
            department,
            week,
            content,
            status: ReviewStatus::Draft,
        }
    }

    /// Reconstitute a report from already-persisted parts, including its
    /// [`ReviewStatus`].
    ///
    /// Unlike [`Report::new`] — which always starts a fresh [`Draft`] — this
    /// rebuilds a report in *any* state without replaying the workflow. It is the
    /// entry point a [`ReportStorage`](crate::ports::reportstorage::ReportStorage)
    /// adapter uses to load a stored row back into memory: the state was already
    /// validated when it was first written. It is deliberately **not** a way for
    /// application logic to bypass the [`submit`]/[`sign`]/[`reject`] rules — drive
    /// those through the workflow methods instead.
    ///
    /// [`Draft`]: ReviewStatus::Draft
    /// [`submit`]: Report::submit
    /// [`sign`]: Report::sign
    /// [`reject`]: Report::reject
    pub fn from_parts(
        id: ReportId,
        author: UserId,
        department: DepartmentId,
        week: IsoWeek,
        content: String,
        status: ReviewStatus,
    ) -> Self {
        Self {
            id,
            author,
            department,
            week,
            content,
            status,
        }
    }

    /// Replace the markdown body. Allowed while the report is a [`Draft`], still
    /// [`Submitted`], or has been [`Rejected`]; revising returns it to [`Draft`]
    /// for (re)submission.
    ///
    /// [`Draft`]: ReviewStatus::Draft
    /// [`Submitted`]: ReviewStatus::Submitted
    /// [`Rejected`]: ReviewStatus::Rejected
    pub fn revise(&mut self, content: String) -> Result<(), DomainError> {
        match self.status {
            ReviewStatus::Draft
            | ReviewStatus::Submitted { .. }
            | ReviewStatus::Rejected { .. } => {
                self.content = content;
                self.status = ReviewStatus::Draft;
                Ok(())
            }
            ReviewStatus::Signed { .. } => Err(DomainError::Invalid(
                "cannot revise a report that has been signed".into(),
            )),
        }
    }

    /// Submit the report into its department's queue. Allowed from [`Draft`] or
    /// [`Rejected`] (a straight resubmission); the body must not be empty, and the
    /// covered week must not be in the future relative to `at`.
    ///
    /// A report may only be submitted for the current week or an earlier one — a
    /// trainee can file a late report after the week is over, but not get ahead of
    /// the calendar. "Current" is the ISO week containing `at`, evaluated in **UTC**
    /// (so near the Sun→Mon UTC boundary a positive-offset caller may briefly be
    /// blocked on a week they consider current).
    ///
    /// [`Draft`]: ReviewStatus::Draft
    /// [`Rejected`]: ReviewStatus::Rejected
    pub fn submit(&mut self, at: Timestamp) -> Result<(), DomainError> {
        match self.status {
            ReviewStatus::Draft | ReviewStatus::Rejected { .. } => {
                if self.content.trim().is_empty() {
                    return Err(DomainError::Invalid("cannot submit an empty report".into()));
                }
                if self.week.is_after(&IsoWeek::from_timestamp_utc(at)) {
                    return Err(DomainError::Invalid(
                        "cannot submit a report for a future week".into(),
                    ));
                }
                self.status = ReviewStatus::Submitted { at };
                Ok(())
            }
            ReviewStatus::Submitted { .. } => Err(DomainError::Invalid(
                "report is already awaiting a signature".into(),
            )),
            ReviewStatus::Signed { .. } => Err(DomainError::Invalid(
                "cannot resubmit a report that has been signed".into(),
            )),
        }
    }

    /// Sign the report on behalf of signer `by`. Only valid while it is
    /// [`Submitted`].
    ///
    /// [`Submitted`]: ReviewStatus::Submitted
    pub fn sign(&mut self, by: UserId, at: Timestamp) -> Result<(), DomainError> {
        match self.status {
            ReviewStatus::Submitted { .. } => {
                self.status = ReviewStatus::Signed { at, by };
                Ok(())
            }
            _ => Err(DomainError::Invalid(
                "can only sign a report that is awaiting a signature".into(),
            )),
        }
    }

    /// Reject the report with a reason. Only valid while it is [`Submitted`]; the
    /// reason must not be empty.
    ///
    /// [`Submitted`]: ReviewStatus::Submitted
    pub fn reject(&mut self, reason: String, at: Timestamp) -> Result<(), DomainError> {
        match self.status {
            ReviewStatus::Submitted { .. } => {
                if reason.trim().is_empty() {
                    return Err(DomainError::Invalid(
                        "a rejection must carry a reason".into(),
                    ));
                }
                self.status = ReviewStatus::Rejected { at, reason };
                Ok(())
            }
            _ => Err(DomainError::Invalid(
                "can only reject a report that is awaiting a signature".into(),
            )),
        }
    }

    /// Apply a signer's [`ReviewDecision`], dispatching to [`sign`] or [`reject`].
    /// `signer` is recorded as the signature when the decision is to sign.
    ///
    /// [`sign`]: Report::sign
    /// [`reject`]: Report::reject
    pub fn review(
        &mut self,
        signer: UserId,
        decision: ReviewDecision,
        at: Timestamp,
    ) -> Result<(), DomainError> {
        match decision {
            ReviewDecision::Sign => self.sign(signer, at),
            ReviewDecision::Reject { reason } => self.reject(reason, at),
        }
    }

    /// This report's stable identity.
    pub fn id(&self) -> &ReportId {
        &self.id
    }

    /// The trainee who wrote the report.
    pub fn author(&self) -> &UserId {
        &self.author
    }

    /// The department whose queue this report belongs to.
    pub fn department(&self) -> &DepartmentId {
        &self.department
    }

    /// The ISO week this report covers.
    pub fn week(&self) -> &IsoWeek {
        &self.week
    }

    /// The current markdown body.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Where the report sits in the review workflow.
    pub fn status(&self) -> &ReviewStatus {
        &self.status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Report {
        Report::new(
            ReportId::new("r1"),
            UserId::new("trainee"),
            DepartmentId::new("blue"),
            IsoWeek::new(2026, 24).unwrap(),
            "# Week 1\n\nDid things.".to_owned(),
        )
    }

    fn signer() -> UserId {
        UserId::new("signer")
    }

    /// A fixed instant inside ISO week 2026-W24 (Mon 2026-06-08 .. Sun 2026-06-14),
    /// matching the `draft()` fixture's week. Pinning "now" keeps the submit guard's
    /// future-week check deterministic regardless of when the suite runs.
    fn now() -> Timestamp {
        "2026-06-10T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn new_report_starts_as_draft() {
        assert!(matches!(draft().status(), ReviewStatus::Draft));
    }

    #[test]
    fn happy_path_draft_submit_sign() {
        let mut r = draft();
        r.submit(now()).unwrap();
        assert!(matches!(r.status(), ReviewStatus::Submitted { .. }));
        r.sign(signer(), now()).unwrap();
        match r.status() {
            ReviewStatus::Signed { by, .. } => assert_eq!(by.as_str(), "signer"),
            other => panic!("expected Signed, got {other:?}"),
        }
    }

    #[test]
    fn reject_revise_resubmit_sign_loop() {
        let mut r = draft();
        r.submit(now()).unwrap();
        r.reject("needs more detail".to_owned(), now()).unwrap();
        match r.status() {
            ReviewStatus::Rejected { reason, .. } => assert_eq!(reason, "needs more detail"),
            other => panic!("expected Rejected, got {other:?}"),
        }

        // Trainee addresses the feedback and tries again.
        r.revise("# Week 1\n\nDid things, in detail.".to_owned())
            .unwrap();
        assert!(matches!(r.status(), ReviewStatus::Draft));
        r.submit(now()).unwrap();
        r.sign(signer(), now()).unwrap();
        assert!(matches!(r.status(), ReviewStatus::Signed { .. }));
    }

    #[test]
    fn can_resubmit_directly_from_rejected() {
        let mut r = draft();
        r.submit(now()).unwrap();
        r.reject("redo".to_owned(), now()).unwrap();
        r.submit(now()).unwrap();
        assert!(matches!(r.status(), ReviewStatus::Submitted { .. }));
    }

    #[test]
    fn cannot_submit_empty_content() {
        let mut r = Report::new(
            ReportId::new("r1"),
            UserId::new("t"),
            DepartmentId::new("blue"),
            IsoWeek::new(2026, 24).unwrap(),
            "   \n  ".to_owned(),
        );
        assert!(matches!(r.submit(now()), Err(DomainError::Invalid(_))));
    }

    /// Build a draft filed for `week` (otherwise like [`draft`]).
    fn draft_for(week: IsoWeek) -> Report {
        Report::new(
            ReportId::new("r1"),
            UserId::new("trainee"),
            DepartmentId::new("blue"),
            week,
            "# Week\n\nDid things.".to_owned(),
        )
    }

    #[test]
    fn cannot_submit_a_future_week() {
        // `now()` is in 2026-W24, so W25 is one week into the future.
        let mut r = draft_for(IsoWeek::new(2026, 25).unwrap());
        assert!(matches!(r.submit(now()), Err(DomainError::Invalid(_))));
        // The report stays a draft — a failed submit must not advance it.
        assert!(matches!(r.status(), ReviewStatus::Draft));
    }

    #[test]
    fn can_submit_the_current_week() {
        // The boundary is inclusive: the current week (W24, same as `now()`) is fine.
        let mut r = draft_for(IsoWeek::new(2026, 24).unwrap());
        r.submit(now()).unwrap();
        assert!(matches!(r.status(), ReviewStatus::Submitted { .. }));
    }

    #[test]
    fn can_submit_a_past_week() {
        // No lower bound: a late report for an earlier week (W23) still submits.
        let mut r = draft_for(IsoWeek::new(2026, 23).unwrap());
        r.submit(now()).unwrap();
        assert!(matches!(r.status(), ReviewStatus::Submitted { .. }));
    }

    #[test]
    fn cannot_resubmit_a_future_week_after_rejection() {
        // The guard lives in the shared Draft | Rejected arm, so a rejected report
        // for a future week is blocked on resubmission too.
        let mut r = Report::from_parts(
            ReportId::new("r1"),
            UserId::new("trainee"),
            DepartmentId::new("blue"),
            IsoWeek::new(2026, 25).unwrap(),
            "# next week".to_owned(),
            ReviewStatus::Rejected {
                at: now(),
                reason: "redo".to_owned(),
            },
        );
        assert!(matches!(r.submit(now()), Err(DomainError::Invalid(_))));
        assert!(matches!(r.status(), ReviewStatus::Rejected { .. }));
    }

    #[test]
    fn cannot_reject_without_a_reason() {
        let mut r = draft();
        r.submit(now()).unwrap();
        assert!(matches!(
            r.reject("   ".to_owned(), now()),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn cannot_submit_twice() {
        let mut r = draft();
        r.submit(now()).unwrap();
        assert!(matches!(r.submit(now()), Err(DomainError::Invalid(_))));
    }

    #[test]
    fn cannot_sign_a_draft() {
        let mut r = draft();
        assert!(matches!(r.sign(signer(), now()), Err(DomainError::Invalid(_))));
    }

    #[test]
    fn cannot_sign_twice() {
        let mut r = draft();
        r.submit(now()).unwrap();
        r.sign(signer(), now()).unwrap();
        assert!(matches!(r.sign(signer(), now()), Err(DomainError::Invalid(_))));
    }

    #[test]
    fn cannot_reject_a_draft() {
        let mut r = draft();
        assert!(matches!(
            r.reject("no".to_owned(), now()),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn can_revise_a_submitted_report() {
        let mut r = draft();
        r.submit(now()).unwrap();
        assert!(matches!(r.revise("x".to_owned()), Ok(())));
    }

    #[test]
    fn cannot_revise_a_signed_report() {
        let mut r = draft();
        r.submit(now()).unwrap();
        r.sign(signer(), now()).unwrap();
        assert!(matches!(
            r.revise("x".to_owned()),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn review_decision_dispatches_to_sign_and_reject() {
        let mut r = draft();
        r.submit(now()).unwrap();
        r.review(
            signer(),
            ReviewDecision::Reject {
                reason: "fix typos".to_owned(),
            },
            now(),
        )
        .unwrap();
        assert!(matches!(r.status(), ReviewStatus::Rejected { .. }));

        r.revise("fixed".to_owned()).unwrap();
        r.submit(now()).unwrap();
        r.review(signer(), ReviewDecision::Sign, now()).unwrap();
        assert!(matches!(r.status(), ReviewStatus::Signed { .. }));
    }
}
