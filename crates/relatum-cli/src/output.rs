//! Rendering of command results.
//!
//! Every command produces either a typed value (rendered as raw JSON or a few human
//! lines) or a bare acknowledgement for the endpoints that return no body. The JSON
//! arm prints the server's wire shape verbatim — the generated DTOs are `Serialize` —
//! so it is suitable for piping to `jq` while the API is still moving.

use clap::ValueEnum;
use relatum_client::{
    ApiInfo, MeView, ReportView, ReviewStatusDto, RoleDto, SsoInfo, UserSummary,
};
use serde::Serialize;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    /// Compact human-readable lines.
    Text,
    /// The server's JSON response, pretty-printed.
    Json,
}

/// Render a typed value: its JSON wire form, or `text()` for the human format.
pub fn emit<T: Serialize>(
    fmt: OutputFormat,
    value: &T,
    text: impl FnOnce() -> String,
) -> anyhow::Result<()> {
    match fmt {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
        OutputFormat::Text => println!("{}", text()),
    }
    Ok(())
}

/// Acknowledge a body-less operation (submit, review, logout, …).
pub fn ack(fmt: OutputFormat, message: &str) {
    match fmt {
        OutputFormat::Json => {
            println!("{}", serde_json::json!({ "ok": true, "message": message }))
        }
        OutputFormat::Text => println!("{message}"),
    }
}

/// Report a freshly created id (the only field `create` returns).
pub fn emit_id(fmt: OutputFormat, id: &str) {
    match fmt {
        OutputFormat::Json => println!("{}", serde_json::json!({ "id": id })),
        OutputFormat::Text => println!("{id}"),
    }
}

pub fn me_text(me: &MeView) -> String {
    let role = match &me.role {
        None => "none (no department assigned)".to_string(),
        Some(role) => role_text(role),
    };
    format!("id:   {}\nrole: {}", me.id, role)
}

fn role_text(role: &RoleDto) -> String {
    match role {
        RoleDto::Instructor(dept) => format!("instructor ({dept})"),
        RoleDto::Trainee(dept) => format!("trainee ({dept})"),
        RoleDto::Signer(dept) => format!("signer ({dept})"),
    }
}

pub fn sso_text(info: &SsoInfo) -> String {
    let login_url = info.login_url.as_deref().unwrap_or("-");
    format!("enabled:   {}\nlogin_url: {login_url}", info.enabled)
}

pub fn info_text(info: &ApiInfo) -> String {
    format!("{} {}", info.name, info.version)
}

pub fn report_text(report: &ReportView) -> String {
    format!(
        "id:         {}\nauthor:     {}\ndepartment: {}\nstatus:     {}\n\n{}",
        report.id,
        report.author,
        report.department,
        status_text(&report.status),
        report.content,
    )
}

pub fn reports_text(reports: &[ReportView]) -> String {
    if reports.is_empty() {
        return "(no reports)".to_string();
    }
    reports
        .iter()
        .map(|r| format!("{}  {:<14}  {}", r.id, status_text(&r.status), r.author))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn users_text(users: &[UserSummary]) -> String {
    if users.is_empty() {
        return "(no users)".to_string();
    }
    users
        .iter()
        .map(|u| {
            let dept = u.department.as_deref().unwrap_or("-");
            let role = u.role.as_ref().map(role_text).unwrap_or_else(|| "inert".to_string());
            format!("{:<12}  {:<10}  {:<8}  {}", u.username, marker_text(&u.marker), dept, role)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn marker_text(marker: &relatum_client::MarkerDto) -> &'static str {
    match marker {
        relatum_client::MarkerDto::Instructor => "instructor",
        relatum_client::MarkerDto::Trainee => "trainee",
        relatum_client::MarkerDto::Regular => "regular",
    }
}

fn status_text(status: &ReviewStatusDto) -> String {
    match status {
        ReviewStatusDto::Draft => "draft".to_string(),
        ReviewStatusDto::Submitted { .. } => "submitted".to_string(),
        ReviewStatusDto::Signed { by, .. } => format!("signed by {by}"),
        ReviewStatusDto::Rejected { reason, .. } => format!("rejected: {reason}"),
    }
}
