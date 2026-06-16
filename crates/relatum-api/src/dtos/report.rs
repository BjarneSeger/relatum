//! Report DTOs — the wire shapes for the trainee → signer review workflow.

use relatum_domain::models::ids::ReportId;
use relatum_domain::models::report::{Report, ReviewDecision, ReviewStatus};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A report as returned to clients.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReportView {
    pub id: String,
    /// The trainee who wrote the report.
    pub author: String,
    /// The department whose queue the report belongs to.
    pub department: String,
    /// The ISO week the report covers, e.g. `2026-W24`.
    pub week: String,
    /// The report body, as markdown.
    pub content: String,
    pub status: ReviewStatusDto,
}

impl From<Report> for ReportView {
    fn from(report: Report) -> Self {
        ReportView {
            id: report.id().as_str().to_owned(),
            author: report.author().as_str().to_owned(),
            department: report.department().as_str().to_owned(),
            week: report.week().to_string(),
            content: report.content().to_owned(),
            status: report.status().into(),
        }
    }
}

/// Where a report sits in the review cycle. Timestamps are RFC3339 strings.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReviewStatusDto {
    Draft,
    Submitted { at: String },
    /// Signed by a signer; `by` is the signer's id.
    Signed { at: String, by: String },
    Rejected { at: String, reason: String },
}

impl From<&ReviewStatus> for ReviewStatusDto {
    fn from(status: &ReviewStatus) -> Self {
        match status {
            ReviewStatus::Draft => Self::Draft,
            ReviewStatus::Submitted { at } => Self::Submitted { at: at.to_string() },
            ReviewStatus::Signed { at, by } => Self::Signed {
                at: at.to_string(),
                by: by.as_str().to_owned(),
            },
            ReviewStatus::Rejected { at, reason } => Self::Rejected {
                at: at.to_string(),
                reason: reason.clone(),
            },
        }
    }
}

/// Body for starting a new draft report.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateReportRequest {
    /// The ISO week the report covers, e.g. `2026-W24`.
    pub week: String,
    /// The report body, as markdown.
    pub content: String,
}

/// Body for replacing a draft/rejected report's markdown.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReviseReportRequest {
    pub content: String,
}

/// A signer's verdict on a submitted report.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ReviewDecisionDto {
    Sign,
    Reject { reason: String },
}

impl From<ReviewDecisionDto> for ReviewDecision {
    fn from(decision: ReviewDecisionDto) -> Self {
        match decision {
            ReviewDecisionDto::Sign => ReviewDecision::Sign,
            ReviewDecisionDto::Reject { reason } => ReviewDecision::Reject { reason },
        }
    }
}

/// Body posted to the review endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReviewRequest {
    pub decision: ReviewDecisionDto,
}

/// Response to creating a draft: the fresh report's id.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatedReport {
    pub id: String,
}

impl From<ReportId> for CreatedReport {
    fn from(id: ReportId) -> Self {
        CreatedReport {
            id: id.as_str().to_owned(),
        }
    }
}
