//! `relatum` — a thin CLI over the relatum HTTP API.
//!
//! This frontend exists to drive and test a running server while the API is still
//! moving: every subcommand maps to a single [`relatum_client::Client`] call, so the
//! CLI owns no HTTP of its own. Its only jobs are to parse arguments, resolve the
//! session token (a `--token`/env value, or the one `login` saved to the keyring), and
//! render the result as human text or raw JSON.
//!
//! Authentication is SSO-only on the server: `login` takes an SSO access token,
//! exchanges it for a session token, and persists that token (see [`token`]) so later
//! commands reuse it.

mod output;
mod token;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use relatum_client::{Client, ReviewDecisionDto, SignatureFormatDto};

use crate::output::OutputFormat;

/// Thin CLI over the relatum HTTP API.
#[derive(Parser)]
#[command(name = "relatum", version, about, long_about = None)]
struct Cli {
    /// Base URL of the relatum server.
    #[arg(
        short,
        long,
        env = "RELATUM_URL",
        default_value = "http://localhost:8080",
        global = true
    )]
    url: String,

    /// Session bearer token. Overrides the token saved by `login`; falls back to it
    /// when unset.
    #[arg(short, long, env = "RELATUM_TOKEN", global = true)]
    token: Option<String>,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Text, global = true)]
    output: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Exchange an SSO access token for a session token and store it.
    Login {
        /// The SSO access token obtained from the identity provider.
        sso_token: String,
    },
    /// Revoke the session server-side and forget the stored token.
    Logout,
    /// Rotate the stored session token.
    Refresh,
    /// Show the authenticated caller's identity and role.
    Me,
    /// Show whether the server offers SSO login, and where the flow starts.
    SsoInfo,
    /// Create, inspect and move reports through their workflow.
    #[command(subcommand)]
    Reports(ReportsCommand),
    /// Register or view your signature (required before you can submit or sign).
    #[command(subcommand)]
    Signature(SignatureCommand),
    /// User administration (instructor-only).
    #[command(subcommand)]
    Users(UsersCommand),
    /// Service metadata and health probes.
    #[command(subcommand)]
    Meta(MetaCommand),
}

#[derive(Subcommand)]
enum ReportsCommand {
    /// Start a draft report.
    Create {
        /// The ISO week the report covers, e.g. `2026-W24`.
        #[arg(long)]
        week: String,
        /// Markdown content inline.
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,
        /// Read markdown content from a file, or `-` for stdin.
        #[arg(long, conflicts_with = "content")]
        file: Option<PathBuf>,
    },
    /// List the caller's reports (authored, or the review queue, per role).
    List,
    /// Show a single report.
    Get { id: String },
    /// Replace a draft/rejected report's markdown.
    Revise {
        id: String,
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,
        #[arg(long, conflicts_with = "content")]
        file: Option<PathBuf>,
    },
    /// Submit a report into its department's queue.
    Submit { id: String },
    /// Sign or reject a submitted report (signers in its department only).
    Review {
        id: String,
        #[command(subcommand)]
        decision: ReviewDecision,
    },
    /// Export a report as a signed PDF (Ausbildungsnachweis).
    Export {
        id: String,
        /// Output file. Defaults to `ausbildungsnachweis-<id>.pdf`; use `-` for stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ReviewDecision {
    /// Sign the report.
    Sign,
    /// Reject the report with a reason.
    Reject {
        #[arg(long)]
        reason: String,
    },
}

#[derive(Subcommand)]
enum SignatureCommand {
    /// Set or replace your signature from a PNG file (or `-` for stdin).
    Set {
        /// Path to a PNG image, or `-` to read PNG bytes from stdin.
        #[arg(long)]
        file: PathBuf,
    },
    /// Show your signature's format and when it was last set.
    Show,
}

#[derive(Subcommand)]
enum UsersCommand {
    /// List every user the instance knows about (instructor-only).
    List,
    /// Assign a user to a department (turns a regular user into a signer).
    AssignDept { user_id: String, department: String },
    /// Clear a user's department, returning them to the inert state.
    ClearDept { user_id: String },
}

#[derive(Subcommand)]
enum MetaCommand {
    /// Name and version of the running service.
    Info,
    /// Liveness probe.
    Healthz,
    /// Readiness probe (checks the backing stores).
    Readyz,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let fmt = cli.output;

    // An explicit --token / RELATUM_TOKEN wins; otherwise resume the saved session.
    let token = match cli.token {
        Some(t) => Some(t),
        None => token::load()?,
    };
    let mut client = Client::with_token(cli.url, token);

    match cli.command {
        Command::Login { sso_token } => {
            client.login(&sso_token).await?;
            token::store(
                client
                    .token()
                    .expect("token is set after a successful login"),
            )?;
            output::ack(fmt, "logged in");
        }
        Command::Logout => {
            client.logout().await?;
            token::clear()?;
            output::ack(fmt, "logged out");
        }
        Command::Refresh => {
            client.refresh().await?;
            token::store(
                client
                    .token()
                    .expect("token is set after a successful refresh"),
            )?;
            output::ack(fmt, "token refreshed");
        }
        Command::Me => {
            let me = client.me().await?;
            output::emit(fmt, &me, || output::me_text(&me))?;
        }
        Command::SsoInfo => {
            let info = client.sso_info().await?;
            output::emit(fmt, &info, || output::sso_text(&info))?;
        }
        Command::Reports(cmd) => run_reports(&client, fmt, cmd).await?,
        Command::Signature(cmd) => run_signature(&client, fmt, cmd).await?,
        Command::Users(cmd) => run_users(&client, fmt, cmd).await?,
        Command::Meta(cmd) => run_meta(&client, fmt, cmd).await?,
    }
    Ok(())
}

async fn run_reports(client: &Client, fmt: OutputFormat, cmd: ReportsCommand) -> Result<()> {
    match cmd {
        ReportsCommand::Create {
            week,
            content,
            file,
        } => {
            let body = read_content(content, file)?;
            let id = client.create_report(&week, &body).await?;
            output::emit_id(fmt, &id);
        }
        ReportsCommand::List => {
            let reports = client.list_reports().await?;
            output::emit(fmt, &reports, || output::reports_text(&reports))?;
        }
        ReportsCommand::Get { id } => {
            let report = client.get_report(&id).await?;
            output::emit(fmt, &report, || output::report_text(&report))?;
        }
        ReportsCommand::Revise { id, content, file } => {
            let body = read_content(content, file)?;
            client.revise_report(&id, &body).await?;
            output::ack(fmt, "revised");
        }
        ReportsCommand::Submit { id } => {
            client.submit_report(&id).await?;
            output::ack(fmt, "submitted");
        }
        ReportsCommand::Review { id, decision } => {
            let dto = match decision {
                ReviewDecision::Sign => ReviewDecisionDto::Sign,
                ReviewDecision::Reject { reason } => ReviewDecisionDto::Reject(reason),
            };
            client.review_report(&id, dto).await?;
            output::ack(fmt, "reviewed");
        }
        ReportsCommand::Export { id, output } => {
            let bytes = client.export_report(&id).await?;
            match output {
                Some(path) if path.as_os_str() == "-" => {
                    std::io::stdout()
                        .write_all(&bytes)
                        .context("writing PDF to stdout")?;
                }
                Some(path) => {
                    std::fs::write(&path, &bytes)
                        .with_context(|| format!("writing {}", path.display()))?;
                    output::ack(fmt, &format!("wrote {}", path.display()));
                }
                None => {
                    let name = format!("ausbildungsnachweis-{id}.pdf");
                    std::fs::write(&name, &bytes).with_context(|| format!("writing {name}"))?;
                    output::ack(fmt, &format!("wrote {name}"));
                }
            }
        }
    }
    Ok(())
}

async fn run_signature(client: &Client, fmt: OutputFormat, cmd: SignatureCommand) -> Result<()> {
    match cmd {
        SignatureCommand::Set { file } => {
            let bytes = read_bytes(&file)?;
            // PNG is the only supported format; the server re-validates the magic bytes.
            client
                .set_signature(SignatureFormatDto::Png, &bytes)
                .await?;
            output::ack(fmt, "signature set");
        }
        SignatureCommand::Show => match client.get_signature().await? {
            Some(sig) => output::emit(fmt, &sig, || output::signature_text(&sig))?,
            None => output::ack(fmt, "no signature on file"),
        },
    }
    Ok(())
}

async fn run_users(client: &Client, fmt: OutputFormat, cmd: UsersCommand) -> Result<()> {
    match cmd {
        UsersCommand::List => {
            let users = client.list_users().await?;
            output::emit(fmt, &users, || output::users_text(&users))?;
        }
        UsersCommand::AssignDept {
            user_id,
            department,
        } => {
            client.assign_department(&user_id, &department).await?;
            output::ack(fmt, "department assigned");
        }
        UsersCommand::ClearDept { user_id } => {
            client.clear_department(&user_id).await?;
            output::ack(fmt, "department cleared");
        }
    }
    Ok(())
}

async fn run_meta(client: &Client, fmt: OutputFormat, cmd: MetaCommand) -> Result<()> {
    match cmd {
        MetaCommand::Info => {
            let info = client.info().await?;
            output::emit(fmt, &info, || output::info_text(&info))?;
        }
        MetaCommand::Healthz => {
            client.healthz().await?;
            output::ack(fmt, "alive");
        }
        MetaCommand::Readyz => {
            client.readyz().await?;
            output::ack(fmt, "ready");
        }
    }
    Ok(())
}

/// Resolve report markdown from `--content`, `--file <path>`, or `--file -` (stdin).
fn read_content(content: Option<String>, file: Option<PathBuf>) -> Result<String> {
    match (content, file) {
        (Some(c), _) => Ok(c),
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading content from stdin")?;
            Ok(buf)
        }
        (None, Some(path)) => std::fs::read_to_string(&path)
            .with_context(|| format!("reading content from {}", path.display())),
        (None, None) => bail!("provide --content <text> or --file <path> (use - for stdin)"),
    }
}

/// Read raw bytes from `path`, or from stdin when `path` is `-`. The binary sibling of
/// [`read_content`], used to load a signature image.
fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .context("reading signature from stdin")?;
        Ok(buf)
    } else {
        std::fs::read(path).with_context(|| format!("reading signature from {}", path.display()))
    }
}
