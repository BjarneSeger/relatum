//! Typed HTTP client for `relatum-server`, generated from its OpenAPI document.
//!
//! The [`generated`] module is produced at build time by `build.rs` from
//! `relatum_api::openapi_json()`, so its request methods and types track the server
//! exactly — change a route or DTO and the client regenerates on the next build.
//!
//! Application code should use the hand-written [`Client`] wrapper below: it owns the
//! base URL and bearer token, exposes ergonomic role-oriented methods, and maps the
//! generated `Result`s onto a single [`ClientError`]. The generated request/response
//! types ([`ReportView`], [`MeView`], [`RoleDto`], …) are re-exported for callers.

/// The progenitor-generated client and types. Accessed through the [`Client`]
/// wrapper rather than directly.
pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
}

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use generated::types;

/// Wire types callers (the CLI and web frontend) consume directly. These are the
/// *generated* shapes, so they cannot drift from the server contract.
pub use generated::types::{
    ApiInfo, MarkerDto, MeView, ReportView, ReviewDecisionDto, ReviewStatusDto, RoleDto,
    SetSignatureRequest, SignatureFormatDto, SignatureView, SsoInfo, UserSummary,
};

/// Anything that can go wrong talking to the server.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The request never produced a valid HTTP response (DNS, TLS, connection,
    /// timeout, malformed URL, …).
    #[error("could not reach the server: {0}")]
    Transport(String),
    /// The server answered with a non-success status and a structured error body.
    #[error("server returned {status}: {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    /// The server answered, but not in any shape the contract anticipates — an
    /// undocumented status code, or a body that failed to deserialize into the
    /// expected type. The detail names which, so it can be logged and diagnosed.
    #[error("unexpected response from the server: {0}")]
    Unexpected(String),
}

impl From<generated::Error<types::ErrorResponse>> for ClientError {
    fn from(err: generated::Error<types::ErrorResponse>) -> Self {
        use generated::Error;
        match err {
            Error::ErrorResponse(rv) => {
                let status = rv.status().as_u16();
                let body = rv.into_inner();
                ClientError::Api {
                    status,
                    code: body.code,
                    message: body.message,
                }
            }
            Error::CommunicationError(e) => ClientError::Transport(e.to_string()),
            Error::InvalidRequest(msg) => ClientError::Transport(msg),
            Error::InvalidUpgrade(e) | Error::ResponseBodyError(e) => {
                ClientError::Transport(e.to_string())
            }
            // The expected status came back, but its body didn't fit the schema
            // (a contract drift between server and generated client). The bytes
            // and serde error are the whole story for diagnosing it.
            Error::InvalidResponsePayload(body, e) => ClientError::Unexpected(format!(
                "response body did not match the expected schema ({e}); body was: {}",
                String::from_utf8_lossy(&body)
            )),
            // A status the operation never declared. We can read the status code
            // synchronously; the body would need an await this `From` can't do.
            Error::UnexpectedResponse(resp) => {
                ClientError::Unexpected(format!("undocumented HTTP {} response", resp.status()))
            }
            other => ClientError::Unexpected(other.to_string()),
        }
    }
}

/// A session-aware client for `relatum-server`.
///
/// Holds the base URL and (once logged in) the bearer token. It is cheap to
/// [`Clone`] — only two strings — so callers can hand copies to concurrent tasks.
/// Each call builds a short-lived `reqwest::Client` carrying the auth header; for
/// this app's modest request volume that is simpler than sharing a pooled client and
/// re-attaching the token per request.
#[derive(Clone, Debug)]
pub struct Client {
    base_url: String,
    token: Option<String>,
}

impl Client {
    /// A client with no session yet — only [`login`](Self::login) is meaningful
    /// until a token is set.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
        }
    }

    /// A client that resumes an existing session (e.g. a token restored from the
    /// on-disk session file or an HttpOnly session cookie).
    pub fn with_token(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token,
        }
    }

    /// The current session token, if logged in.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Replace (or clear, with `None`) the session token.
    pub fn set_token(&mut self, token: Option<String>) {
        self.token = token;
    }

    /// Whether a session token is currently held.
    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// Build a generated client, attaching the bearer token as a default header when
    /// present.
    fn api(&self) -> Result<generated::Client, ClientError> {
        let mut builder = reqwest::Client::builder();
        if let Some(token) = &self.token {
            let mut headers = reqwest::header::HeaderMap::new();
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| ClientError::Transport(e.to_string()))?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
            builder = builder.default_headers(headers);
        }
        let http = builder
            .build()
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        Ok(generated::Client::new_with_client(&self.base_url, http))
    }

    /// Exchange an SSO access token for a session token, storing it on success.
    ///
    /// Authentication is SSO-only: the caller obtains the access token from the
    /// external provider (see [`sso_info`](Self::sso_info)) and presents it here.
    pub async fn login(&mut self, sso_token: &str) -> Result<(), ClientError> {
        // Login needs no prior token, so use an unauthenticated client.
        let api = generated::Client::new(&self.base_url);
        let req = types::LoginRequest {
            token: sso_token.to_string(),
        };
        let token = api.login(&req).await?.into_inner().0.token;
        self.token = Some(token);
        Ok(())
    }

    /// Redeem a single-use SSO handoff code for a session token, storing it on success.
    ///
    /// Used by a browser frontend to finish the SSO flow back-channel: it catches the
    /// `code` the SSO callback redirected it with and swaps it here for the session,
    /// so the token never travels in a URL. Needs no prior token.
    pub async fn exchange_sso(&mut self, code: &str) -> Result<(), ClientError> {
        let api = generated::Client::new(&self.base_url);
        let req = types::SsoExchangeRequest {
            code: code.to_string(),
        };
        let token = api.sso_exchange(&req).await?.into_inner().0.token;
        self.token = Some(token);
        Ok(())
    }

    /// Revoke the current session server-side (best effort) and drop the local token.
    pub async fn logout(&mut self) -> Result<(), ClientError> {
        if self.token.is_some() {
            // Ignore failures: the goal is to forget the session locally regardless.
            let _ = self.api()?.logout().await;
        }
        self.token = None;
        Ok(())
    }

    /// Rotate the current session token, replacing the held one on success.
    pub async fn refresh(&mut self) -> Result<(), ClientError> {
        let token = self.api()?.refresh().await?.into_inner().0.token;
        self.token = Some(token);
        Ok(())
    }

    /// The authenticated caller's identity and role — used to pick the UI mode.
    pub async fn me(&self) -> Result<MeView, ClientError> {
        Ok(self.api()?.me().await?.into_inner())
    }

    /// Whether the server offers SSO login, and where the browser flow starts. Needs
    /// no token, so it uses an unauthenticated client (like [`login`](Self::login)).
    pub async fn sso_info(&self) -> Result<SsoInfo, ClientError> {
        // This endpoint declares no error body, so the generated method carries
        // `Error<()>` rather than `Error<ErrorResponse>`; map it by hand.
        let api = generated::Client::new(&self.base_url);
        match api.sso_info().await {
            Ok(resp) => Ok(resp.into_inner()),
            Err(generated::Error::CommunicationError(e)) => {
                Err(ClientError::Transport(e.to_string()))
            }
            Err(generated::Error::InvalidRequest(msg)) => Err(ClientError::Transport(msg)),
            Err(generated::Error::InvalidUpgrade(e) | generated::Error::ResponseBodyError(e)) => {
                Err(ClientError::Transport(e.to_string()))
            }
            Err(generated::Error::UnexpectedResponse(resp)) => Err(ClientError::Api {
                status: resp.status().as_u16(),
                code: "unexpected".to_owned(),
                message: "SSO availability request failed".to_owned(),
            }),
            Err(other) => Err(ClientError::Unexpected(other.to_string())),
        }
    }

    /// The caller's reports: authored ones for a trainee, the review queue for an
    /// instructor (the server decides based on role).
    pub async fn list_reports(&self) -> Result<Vec<ReportView>, ClientError> {
        Ok(self.api()?.list().await?.into_inner())
    }

    /// A single report visible to the caller.
    pub async fn get_report(&self, id: &str) -> Result<ReportView, ClientError> {
        Ok(self.api()?.get(id).await?.into_inner())
    }

    /// Download a report rendered as a signed PDF (Ausbildungsnachweis), authorized
    /// by the same visibility rule as [`get_report`](Self::get_report).
    ///
    /// This endpoint returns an `application/pdf` body, which the OpenAPI-generated
    /// client does not model (binary responses do not survive codegen — the same
    /// reason signatures are base64-in-JSON). So it is fetched with a direct request
    /// here, reusing the wrapper's base URL and bearer token, and the error body is
    /// mapped onto [`ClientError`] just like the generated calls.
    pub async fn export_report(&self, id: &str) -> Result<Vec<u8>, ClientError> {
        // Report ids are server-minted opaque tokens, so the path needs no escaping.
        let url = format!("{}/api/v1/reports/{id}/export", self.base_url);
        let mut request = reqwest::Client::new().get(&url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;

        let status = response.status();
        if status.is_success() {
            let bytes = response
                .bytes()
                .await
                .map_err(|e| ClientError::Transport(e.to_string()))?;
            return Ok(bytes.to_vec());
        }
        // Mirror the generated error mapping: surface the structured body if present.
        let code = status.as_u16();
        match response.json::<types::ErrorResponse>().await {
            Ok(body) => Err(ClientError::Api {
                status: code,
                code: body.code,
                message: body.message,
            }),
            Err(_) => Err(ClientError::Api {
                status: code,
                code: "error".to_owned(),
                message: format!("export request failed with status {code}"),
            }),
        }
    }

    /// Start a new draft report covering `week` (ISO `YYYY-Www`), returning its
    /// server id.
    pub async fn create_report(&self, week: &str, content: &str) -> Result<String, ClientError> {
        let req = types::CreateReportRequest {
            week: week.to_string(),
            content: content.to_string(),
        };
        Ok(self.api()?.create(&req).await?.into_inner().id)
    }

    /// Replace a draft/rejected report's markdown.
    pub async fn revise_report(&self, id: &str, content: &str) -> Result<(), ClientError> {
        let req = types::ReviseReportRequest {
            content: content.to_string(),
        };
        self.api()?.revise(id, &req).await?;
        Ok(())
    }

    /// Submit a report into its department's queue.
    pub async fn submit_report(&self, id: &str) -> Result<(), ClientError> {
        self.api()?.submit(id).await?;
        Ok(())
    }

    /// Sign or reject a submitted report (signers in the report's department only).
    pub async fn review_report(
        &self,
        id: &str,
        decision: ReviewDecisionDto,
    ) -> Result<(), ClientError> {
        let req = types::ReviewRequest { decision };
        self.api()?.review(id, &req).await?;
        Ok(())
    }

    /// Set or replace the caller's own signature from raw image bytes.
    ///
    /// A trainee or signer must register one before they can submit or sign a report
    /// (the server rejects instructors and inert users). The bytes are base64-encoded
    /// here and sent as JSON, so callers pass the raw image.
    pub async fn set_signature(
        &self,
        format: SignatureFormatDto,
        image: &[u8],
    ) -> Result<(), ClientError> {
        let req = types::SetSignatureRequest {
            format,
            data_base64: BASE64.encode(image),
        };
        self.api()?.set_signature(&req).await?;
        Ok(())
    }

    /// The caller's signature, or `None` if they have not registered one.
    pub async fn get_signature(&self) -> Result<Option<SignatureView>, ClientError> {
        match self.api()?.get_signature().await {
            Ok(resp) => Ok(Some(resp.into_inner())),
            // "No signature on file" is the documented 404 — a normal absence, not an
            // error to surface to the caller.
            Err(err) => match ClientError::from(err) {
                ClientError::Api { status: 404, .. } => Ok(None),
                other => Err(other),
            },
        }
    }

    /// Every user the instance knows about (instructor-only on the server).
    ///
    /// Lets an admin frontend present users by name and key its assign/clear actions
    /// off the returned ids, so no one has to type a raw user id.
    pub async fn list_users(&self) -> Result<Vec<UserSummary>, ClientError> {
        Ok(self.api()?.list_users().await?.into_inner())
    }

    /// Assign a user to a department — the mutation that turns a regular directory
    /// user into a signer (instructor-only on the server).
    pub async fn assign_department(
        &self,
        user_id: &str,
        department: &str,
    ) -> Result<(), ClientError> {
        let req = types::AssignDepartmentRequest {
            department: department.to_string(),
        };
        self.api()?.assign_department(user_id, &req).await?;
        Ok(())
    }

    /// Clear a user's department, returning them to the inert (no-role) state.
    pub async fn clear_department(&self, user_id: &str) -> Result<(), ClientError> {
        self.api()?.clear_department(user_id).await?;
        Ok(())
    }

    /// The running service's name and version. Needs no token.
    pub async fn info(&self) -> Result<ApiInfo, ClientError> {
        let api = generated::Client::new(&self.base_url);
        Ok(api.info().await?.into_inner())
    }

    /// Liveness probe: `Ok(())` once the server has started. Needs no token.
    ///
    /// This endpoint declares no error body, so its generated method carries
    /// `Error<()>` rather than `Error<ErrorResponse>`; map it by hand like
    /// [`sso_info`](Self::sso_info).
    pub async fn healthz(&self) -> Result<(), ClientError> {
        let api = generated::Client::new(&self.base_url);
        match api.healthz().await {
            Ok(_) => Ok(()),
            Err(generated::Error::CommunicationError(e)) => {
                Err(ClientError::Transport(e.to_string()))
            }
            Err(generated::Error::InvalidRequest(msg)) => Err(ClientError::Transport(msg)),
            Err(generated::Error::InvalidUpgrade(e) | generated::Error::ResponseBodyError(e)) => {
                Err(ClientError::Transport(e.to_string()))
            }
            Err(generated::Error::UnexpectedResponse(resp)) => Err(ClientError::Api {
                status: resp.status().as_u16(),
                code: "unexpected".to_owned(),
                message: "liveness probe failed".to_owned(),
            }),
            Err(other) => Err(ClientError::Unexpected(other.to_string())),
        }
    }

    /// Readiness probe: `Ok(())` when the server can serve traffic (its backing
    /// stores answered). Needs no token.
    pub async fn readyz(&self) -> Result<(), ClientError> {
        let api = generated::Client::new(&self.base_url);
        api.readyz().await?;
        Ok(())
    }
}
