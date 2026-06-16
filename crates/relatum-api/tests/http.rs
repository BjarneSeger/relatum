//! End-to-end HTTP tests for the API layer.
//!
//! These wire the **real** domain services over the in-memory port doubles from
//! [`relatum_domain::testing`] (the same fakes the domain's own unit tests use) and
//! drive the router with `tower::ServiceExt::oneshot` — no network, no
//! `relatum-infra`. They exercise the whole stack: extraction, serde
//! (de)serialization, the domain logic, and the `DomainError` → HTTP status mapping.
//!
//! Authentication is SSO-only, so logins present an access token the `MockSSO`
//! provider has been told about; users must already exist locally (the directory
//! sync provisions them in production — here the fixture seeds them directly).

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use relatum_api::AppState;
use relatum_domain::models::department::DepartmentRegistry;
use relatum_domain::models::ids::DepartmentId;
use relatum_domain::models::users::User;
use relatum_domain::ports::sso_connector::SsoIdentity;
use relatum_domain::ports::userstorage::UserStorage;
use relatum_domain::services::admin::UserAdmin;
use relatum_domain::services::auth::Authenticator;
use relatum_domain::services::meta::MetaService;
use relatum_domain::services::report::ReportService;
use relatum_domain::testing::{
    InMemoryReports, InMemorySessions, InMemoryUsers, MockSSO, SeqIds, dev_token,
    dev_user_catalogue,
};

// ----------------------------------------------------------------------------
// Test fixture.
// ----------------------------------------------------------------------------

/// A wired-up app with seeded users in department `blue`: instructor `ins`,
/// trainee `tr`, signer `sig`; plus a signer `out` in the unrelated department
/// `red`, and a department-less regular user `newbie`. Each has an SSO token
/// `tok-<id>` the mock provider attests.
struct Fixture {
    router: Router,
}

impl Fixture {
    async fn new() -> Self {
        let users = InMemoryUsers::default();
        let sso = MockSSO::default();

        for (id, marker, department) in dev_user_catalogue() {
            let username = id.as_str().to_owned();
            sso.register(&dev_token(id.as_str()), SsoIdentity { id: id.clone() });
            users
                .store(User::new(id, username, marker, department))
                .await
                .unwrap();
        }

        let sessions = InMemorySessions::default();
        let reports = InMemoryReports::default();
        let ids = SeqIds::default();

        let state = AppState::new(
            Authenticator::new(users.clone(), sessions.clone(), ids.clone(), sso),
            MetaService::new(
                "relatum",
                "0.1.0",
                users.clone(),
                sessions.clone(),
                reports.clone(),
            ),
            ReportService::new(reports.clone(), users.clone(), ids.clone()),
            UserAdmin::new(
                users.clone(),
                DepartmentRegistry::new([DepartmentId::new("blue"), DepartmentId::new("red")]),
            ),
        );

        Fixture {
            router: relatum_api::router(state),
        }
    }

    /// Send a request and return its status and JSON body (`Null` if empty).
    async fn req(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// Log in via SSO and return the session token.
    async fn login(&self, user: &str) -> String {
        let (status, body) = self
            .req(
                Method::POST,
                "/api/v1/login",
                None,
                Some(json!({ "token": dev_token(user) })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "login failed: {body:?}");
        body["token"].as_str().expect("token in body").to_owned()
    }
}

// ----------------------------------------------------------------------------
// Tests.
// ----------------------------------------------------------------------------

#[test]
fn openapi_spec_covers_every_route() {
    // Derived from the same routes the router serves, so this also guards against
    // any handler/spec drift.
    let spec = relatum_api::spec::openapi_json().unwrap();

    for schema in [
        "ApiInfo",
        "ErrorResponse",
        "MeView",
        "ReportView",
        "ReviewStatusDto",
        "ReviewRequest",
        "RoleDto",
        "AssignDepartmentRequest",
        "UserSummary",
        "MarkerDto",
        "SsoInfo",
        "SsoExchangeRequest",
    ] {
        assert!(spec.contains(schema), "missing schema {schema}");
    }
    for path in [
        "/api/v1/info",
        "/api/v1/healthz",
        "/api/v1/readyz",
        "/api/v1/login",
        "/api/v1/logout",
        "/api/v1/refresh",
        "/api/v1/me",
        "/api/v1/auth/sso",
        "/api/v1/auth/sso/exchange",
        "/api/v1/reports",
        "/api/v1/reports/{id}",
        "/api/v1/reports/{id}/submit",
        "/api/v1/reports/{id}/review",
        "/api/v1/users",
        "/api/v1/users/{id}/department",
    ] {
        assert!(spec.contains(path), "missing path {path}");
    }
}

#[tokio::test]
async fn sso_info_reports_disabled_for_the_mock_provider() {
    // The test fixture wires `MockSSO`, which only validates tokens and drives no
    // browser flow, so the endpoint reports SSO unavailable.
    let fx = Fixture::new().await;
    let (status, body) = fx.req(Method::GET, "/api/v1/auth/sso", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false);
    assert!(body.get("login_url").is_none() || body["login_url"].is_null());
}

#[tokio::test]
async fn sso_exchange_rejects_an_unknown_code() {
    // The mock provider has no handoff stashed, so any code is unknown -> 401. This
    // guards the wiring of the back-channel exchange endpoint.
    let fx = Fixture::new().await;
    let (status, body) = fx
        .req(
            Method::POST,
            "/api/v1/auth/sso/exchange",
            None,
            Some(json!({ "code": "never-issued" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
}

#[tokio::test]
async fn info_reports_name_and_version() {
    let fx = Fixture::new().await;
    let (status, body) = fx.req(Method::GET, "/api/v1/info", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "relatum");
    assert_eq!(body["version"], "0.1.0");
}

#[tokio::test]
async fn health_and_readiness_are_ok() {
    let fx = Fixture::new().await;
    for path in ["/api/v1/healthz", "/api/v1/readyz"] {
        let (status, _) = fx.req(Method::GET, path, None, None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn login_rejects_an_unknown_token() {
    let fx = Fixture::new().await;
    let (status, body) = fx
        .req(
            Method::POST,
            "/api/v1/login",
            None,
            Some(json!({ "token": "forged" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
}

#[tokio::test]
async fn refresh_rotates_the_token_and_logout_revokes_it() {
    let fx = Fixture::new().await;
    let first = fx.login("tr").await;

    let (status, body) = fx
        .req(Method::POST, "/api/v1/refresh", Some(&first), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let second = body["token"].as_str().unwrap().to_owned();
    assert_ne!(first, second, "refresh should mint a new token");

    // The old token is now revoked; the new one still works.
    let (old, _) = fx
        .req(Method::GET, "/api/v1/reports", Some(&first), None)
        .await;
    assert_eq!(old, StatusCode::UNAUTHORIZED);

    let (status, _) = fx
        .req(Method::POST, "/api/v1/logout", Some(&second), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (after, _) = fx
        .req(Method::GET, "/api/v1/reports", Some(&second), None)
        .await;
    assert_eq!(after, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn me_returns_the_callers_identity_and_role() {
    let fx = Fixture::new().await;

    // A trainee sees their own id, role, and department.
    let trainee = fx.login("tr").await;
    let (status, body) = fx
        .req(Method::GET, "/api/v1/me", Some(&trainee), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "tr");
    assert_eq!(body["role"]["role"], "trainee");
    assert_eq!(body["role"]["department"], "blue");

    // A regular user with a department is reported as a signer.
    let signer = fx.login("sig").await;
    let (status, body) = fx.req(Method::GET, "/api/v1/me", Some(&signer), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"]["role"], "signer");

    // A user without a department has no role.
    let newbie = fx.login("newbie").await;
    let (status, body) = fx.req(Method::GET, "/api/v1/me", Some(&newbie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("role").is_none() || body["role"].is_null());

    // Without a token it is rejected.
    let (status, _) = fx.req(Method::GET, "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unauthenticated_requests_are_rejected() {
    let fx = Fixture::new().await;
    let (status, _) = fx.req(Method::GET, "/api/v1/reports", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn report_lifecycle_create_submit_sign() {
    let fx = Fixture::new().await;
    let trainee = fx.login("tr").await;

    // Trainee drafts a report.
    let (status, body) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "2026-W24", "content": "# Week 1\n\nDid things." })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap().to_owned();

    // It starts as a draft in the trainee's department, visible to its author,
    // carrying the week it covers.
    let (status, body) = fx
        .req(
            Method::GET,
            &format!("/api/v1/reports/{id}"),
            Some(&trainee),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"]["state"], "draft");
    assert_eq!(body["department"], "blue");
    assert_eq!(body["week"], "2026-W24");

    // Submit it into the department queue (no chosen reviewer).
    let (status, _) = fx
        .req(
            Method::POST,
            &format!("/api/v1/reports/{id}/submit"),
            Some(&trainee),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A signer in `blue` sees it in their queue and signs it.
    let signer = fx.login("sig").await;
    let (status, body) = fx
        .req(Method::GET, "/api/v1/reports", Some(&signer), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    let (status, _) = fx
        .req(
            Method::POST,
            &format!("/api/v1/reports/{id}/review"),
            Some(&signer),
            Some(json!({ "decision": { "decision": "sign" } })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The trainee now sees it signed, with the signer recorded.
    let (_, body) = fx
        .req(
            Method::GET,
            &format!("/api/v1/reports/{id}"),
            Some(&trainee),
            None,
        )
        .await;
    assert_eq!(body["status"]["state"], "signed");
    assert_eq!(body["status"]["by"], "sig");
}

#[tokio::test]
async fn an_instructor_may_read_but_not_sign() {
    let fx = Fixture::new().await;
    let trainee = fx.login("tr").await;
    let (_, body) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "2026-W24", "content": "# done" })),
        )
        .await;
    let id = body["id"].as_str().unwrap().to_owned();
    fx.req(
        Method::POST,
        &format!("/api/v1/reports/{id}/submit"),
        Some(&trainee),
        None,
    )
    .await;

    let instructor = fx.login("ins").await;
    // An instructor can read any report (global view).
    let (status, _) = fx
        .req(
            Method::GET,
            &format!("/api/v1/reports/{id}"),
            Some(&instructor),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // But cannot sign it.
    let (status, _) = fx
        .req(
            Method::POST,
            &format!("/api/v1/reports/{id}/review"),
            Some(&instructor),
            Some(json!({ "decision": { "decision": "sign" } })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_signer_in_another_department_cannot_see_a_report() {
    let fx = Fixture::new().await;
    let trainee = fx.login("tr").await;
    let (_, body) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "2026-W24", "content": "secret" })),
        )
        .await;
    let id = body["id"].as_str().unwrap().to_owned();

    // `out` is a signer in `red`, not in the report's `blue` queue.
    let outsider = fx.login("out").await;
    let (status, _) = fx
        .req(
            Method::GET,
            &format!("/api/v1/reports/{id}"),
            Some(&outsider),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_second_report_for_the_same_week_conflicts() {
    let fx = Fixture::new().await;
    let trainee = fx.login("tr").await;

    let (status, _) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "2026-W24", "content": "# first" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // Same trainee, same week -> 409 Conflict.
    let (status, body) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "2026-W24", "content": "# dup" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");

    // A different week is accepted.
    let (status, _) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "2026-W25", "content": "# next" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn a_malformed_week_is_rejected() {
    let fx = Fixture::new().await;
    let trainee = fx.login("tr").await;

    let (status, body) = fx
        .req(
            Method::POST,
            "/api/v1/reports",
            Some(&trainee),
            Some(json!({ "week": "not-a-week", "content": "# x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
}

#[tokio::test]
async fn unknown_report_is_not_found() {
    let fx = Fixture::new().await;
    let trainee = fx.login("tr").await;
    let (status, _) = fx
        .req(
            Method::GET,
            "/api/v1/reports/does-not-exist",
            Some(&trainee),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_instructors_may_list_users() {
    let fx = Fixture::new().await;

    // Anonymous and non-instructor callers are refused.
    let (status, _) = fx.req(Method::GET, "/api/v1/users", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    for who in ["tr", "sig"] {
        let token = fx.login(who).await;
        let (status, _) = fx
            .req(Method::GET, "/api/v1/users", Some(&token), None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{who} should be forbidden");
    }

    // An instructor sees the whole catalogue: usernames, markers, and the effective
    // role (null for the department-less `newbie`).
    let instructor = fx.login("ins").await;
    let (status, body) = fx
        .req(Method::GET, "/api/v1/users", Some(&instructor), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let users = body.as_array().expect("an array of users");
    assert_eq!(users.len(), 5);

    let by_id = |id: &str| {
        users
            .iter()
            .find(|u| u["id"] == id)
            .unwrap_or_else(|| panic!("user {id} listed"))
            .clone()
    };

    let tr = by_id("tr");
    assert_eq!(tr["username"], "tr");
    assert_eq!(tr["marker"], "trainee");
    assert_eq!(tr["role"]["role"], "trainee");
    assert_eq!(tr["department"], "blue");

    // `newbie` is inert: a regular marker, but no department and therefore no role.
    let newbie = by_id("newbie");
    assert_eq!(newbie["marker"], "regular");
    assert!(newbie.get("role").is_none() || newbie["role"].is_null());
    assert!(newbie.get("department").is_none() || newbie["department"].is_null());
}

#[tokio::test]
async fn only_instructors_may_assign_a_department() {
    let fx = Fixture::new().await;
    let body = json!({ "department": "blue" });

    // A trainee may not.
    let trainee = fx.login("tr").await;
    let (status, _) = fx
        .req(
            Method::PUT,
            "/api/v1/users/newbie/department",
            Some(&trainee),
            Some(body.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // An instructor may, and `newbie` then shows up as a signer.
    let instructor = fx.login("ins").await;
    let (status, _) = fx
        .req(
            Method::PUT,
            "/api/v1/users/newbie/department",
            Some(&instructor),
            Some(body),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let newbie = fx.login("newbie").await;
    let (_, me) = fx.req(Method::GET, "/api/v1/me", Some(&newbie), None).await;
    assert_eq!(me["role"]["role"], "signer");
    assert_eq!(me["role"]["department"], "blue");
}

#[tokio::test]
async fn assigning_an_unknown_department_is_rejected() {
    let fx = Fixture::new().await;
    let instructor = fx.login("ins").await;
    let (status, _) = fx
        .req(
            Method::PUT,
            "/api/v1/users/newbie/department",
            Some(&instructor),
            Some(json!({ "department": "green" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
