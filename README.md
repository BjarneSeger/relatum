# relatum

A report review and sign-off workflow for training and education settings.

> The server is licensed **AGPL-3.0-only** (the `relatum` CLI is **GPL-3.0-only**). The api is
> under MIT-or-Apache-2.0, so feel free to build anything cool with it.

## Overview

There are three effective roles, derived from a user's directory marker plus a
manually assigned department:

- **Trainee** — drafts and submits reports in their department (one report per ISO week).
- **Signer** — reviews the submitted-report queue for their department and signs or
  rejects each report. A signer is a directory user an instructor has assigned to a
  department.
- **Instructor** — read-only global view of every department's queue, and the only role
  that can assign or clear a user's department.

Users are provisioned from LDAP on a sync interval (markers come from group
membership); department assignments are made in-app and are **preserved across syncs**.
Departments themselves are a fixed allowlist fixed at server startup.

A report moves through a small state machine:

```
          submit                  sign
 Draft ───────────▶ Submitted ───────────▶ Signed
   ▲                  │
   │                  │ reject (with reason)
   │     revise       ▼
   └──────────────  Rejected
```

A rejected report can be revised and resubmitted; a signed report is terminal.

## Quick start (local development)

The repository ships a dev container (`.devcontainer/`) that brings up the toolchain
plus `postgres:18` and `valkey:8` via docker-compose. Open the repo in a
devcontainer-aware editor, or start the stack and exec into the `app` container. Cargo
runs **inside** that container.

The `justfile` provides the day-to-day recipes (run `just` to list them). They run the
server with the `dev` feature, which enables **mock** SSO and directory backends — no
real IdP or LDAP needed:

```sh
just dev    # API (mock auth) + SSR web UI side by side; Ctrl-C stops both
just api    # API server only on :8080 (mock SSO + mock directory)
just web    # web UI only on :8081 (expects the API at http://localhost:8080)
```

With the mock backends, log in using the canned SSO tokens: `tok-ins` (instructor),
`tok-tr` (trainee), `tok-sig` (signer), `tok-out` (a user with no department). The dev
departments are `blue` and `red`.

## Building

```sh
cargo build --release --workspace          # everything
cargo build --release -p relatum-server    # backend only
cargo build --release -p relatum-web       # web UI only
cargo build --release -p relatum-cli       # CLI only
```

## Testing

```sh
cargo test --workspace
```

Unit and HTTP-level tests run the real domain services over in-memory port doubles
(the `relatum-domain` `testing` feature), so no database or external services are
required — this is what CI runs. Postgres integration tests are marked `#[ignore]` and
gated behind a `DATABASE_URL`; run them explicitly against a real database when needed.

## Configuration

The server is configured with layered sources (lowest to highest precedence): built-in
defaults → an optional TOML file (`RELATUM_CONFIG`, or `config.toml`) → `RELATUM_*`
environment variables. Run `relatum-server generate-config` to emit a fully documented
TOML template.

Backends are selected at runtime. `memory` backends keep state in-process (handy for
dev, not replica-safe); production uses `postgres` + `redis`.

### Server (`relatum-server`)

| Variable | Default | Description |
| --- | --- | --- |
| `RELATUM_LISTEN` | `0.0.0.0:8080` | HTTP bind address. |
| `RELATUM_DEPARTMENTS` | `[]` | Comma-separated allowlist of departments. |
| `RELATUM_DATA_BACKEND` | `memory` | `memory` or `postgres`. |
| `RELATUM_DATA_URL` | — | Required for `postgres`, e.g. `postgres://user:pass@host:5432/db`. |
| `RELATUM_SESSIONS_BACKEND` | `memory` | `memory` or `redis`. |
| `RELATUM_SESSIONS_URL` | — | Required for `redis`, e.g. `redis://127.0.0.1:6379`. |
| `RELATUM_SESSIONS_TTL_SECS` | `86400` | Session lifetime, seconds. |
| `RELATUM_SSO_BACKEND` | `disabled` | `disabled` or `oidc`. |
| `RELATUM_SSO_USERINFO_URL` / `_AUTHORIZE_URL` / `_TOKEN_URL` | — | OIDC endpoints (required for `oidc`). |
| `RELATUM_SSO_CLIENT_ID` / `_CLIENT_SECRET` | — | OAuth2 client credentials (required for `oidc`). |
| `RELATUM_SSO_PUBLIC_URL` | — | Server's externally reachable base URL; builds the redirect URI (required for `oidc`). |
| `RELATUM_SSO_SCOPES` | `openid profile groups` | Space-separated OAuth2 scopes. |
| `RELATUM_SSO_ALLOWED_REDIRECTS` | `[]` | Comma-separated origins the browser SSO flow may return to (loopback is always allowed). |
| `RELATUM_DIRECTORY_BACKEND` | `disabled` | `disabled` or `ldap`. |
| `RELATUM_DIRECTORY_URL` | — | LDAP URL (required for `ldap`), e.g. `ldaps://ldap.example:636`. |
| `RELATUM_DIRECTORY_BIND_DN` / `_BIND_PASSWORD` | `""` | Service-account bind (empty = anonymous). |
| `RELATUM_DIRECTORY_USER_BASE` | — | LDAP search base (required for `ldap`). |
| `RELATUM_DIRECTORY_USER_FILTER` | `(objectClass=person)` | User search filter. |
| `RELATUM_DIRECTORY_ID_ATTR` | `uid` | Attribute used as the user id (must match the SSO `sub`). |
| `RELATUM_DIRECTORY_USERNAME_ATTR` | `uid` | Attribute used as the login username. |
| `RELATUM_DIRECTORY_GROUP_ATTR` | `memberOf` | Group-membership attribute. |
| `RELATUM_DIRECTORY_INSTRUCTOR_GROUP` / `_TRAINEE_GROUP` | — | Group DNs mapped to roles (required for `ldap`). |
| `RELATUM_DIRECTORY_SYNC_INTERVAL_SECS` | `3600` | Directory reconcile interval. |
| `RUST_LOG` | — | `tracing` env-filter level. |

### Web UI (`relatum-web`)

| Variable | Default | Description |
| --- | --- | --- |
| `RELATUM_WEB_LISTEN` | `0.0.0.0:8081` | HTTP bind address. |
| `RELATUM_WEB_API_URL` | `http://localhost:8080` | Base URL of the `relatum-server` API. |
| `RELATUM_WEB_PUBLIC_URL` | `http://localhost:8081` | The UI's externally reachable base URL; used to build the SSO `redirect_uri` and to set `Secure` cookies when it is `https://`. Set this to the real public URL in production. |
| `RELATUM_WEB_DEPARTMENTS` | `[]` | Comma-separated departments, mirroring the server's set, for the admin dropdown. |
| `RUST_LOG` | — | `tracing` env-filter level. |

## Deployment

### Container images

`.github/workflows/images.yml` builds both binaries from the single multi-target
`Dockerfile` (selected with `--build-arg BIN=relatum-server` or
`--build-arg BIN=relatum-web`) and pushes them to GitHub Container Registry:

- `ghcr.io/bjarneseger/relatum-server`
- `ghcr.io/bjarneseger/relatum-web`

Tags: `edge-<short-sha>` on pushes to `main`; on a `v*` release tag, the matching
semver tags (`0.1.0` and `0.1`) plus `latest`.

To build an image locally:

```sh
docker build --build-arg BIN=relatum-server -t relatum-server .
docker build --build-arg BIN=relatum-web    -t relatum-web .
```

Health endpoints (used as Kubernetes probes): the server serves
`GET /api/v1/healthz` (liveness) and `GET /api/v1/readyz` (readiness, checks the
backing stores); the web UI serves `GET /healthz`.

### Kubernetes (Helm)

Two charts live under `deploy/helm/`, and are published to
`oci://ghcr.io/bjarneseger/charts` on each release.

Self-contained server install (bundles Postgres and Valkey — no external prerequisites):

```sh
helm install relatum oci://ghcr.io/bjarneseger/charts/relatum-server \
  --version 0.1.0 \
  --set postgresql.enabled=true
```

> A bare `helm install` with no database source **fails on purpose** — with the default
> `config.data.backend=postgres` you must choose where Postgres lives (bundled,
> external, an inline URL, or an existing secret) rather than silently run without
> persistence.

Web UI install, pointed at the server:

```sh
helm install web oci://ghcr.io/bjarneseger/charts/relatum-web \
  --version 0.1.0 \
  --set config.apiUrl=http://relatum-server \
  --set config.publicUrl=https://relatum.example
```

For the full values reference — database/session backend options, OIDC wiring,
replicas, ingress/Gateway API, and the bundled Valkey subchart — see the per-chart
docs: [`relatum-server`](deploy/helm/relatum-server/README.md) and
[`relatum-web`](deploy/helm/relatum-web/README.md).

### CLI install

Tagged releases attach CLI artifacts to the GitHub Release (built via
`.github/workflows/release.yml` and `.goreleaser.yaml`):

- `.tar.xz` archives — Linux `amd64`/`arm64` (musl) and macOS `arm64`
- `.deb` and `.rpm` packages (install the binary to `/usr/bin/relatum`)
- `SHA256SUMS` covering all of the above

Point the CLI at a server (`--url`, or the `RELATUM_URL` env var; default
`http://localhost:8080`) and authenticate by exchanging an SSO access token for a
session token:

```sh
relatum --url https://relatum.example sso-info           # where the SSO flow starts
relatum --url https://relatum.example login <sso-token>
relatum me                                                # who am I / what role
relatum reports create --week 2026-W24 --file report.md
relatum reports submit <id>
relatum reports review <id> sign                          # signers, in their department
relatum users assign-department <user> <department>       # instructors only
```

The session token is kept in the operating system's keyring (the macOS Keychain, or
the Secret Service on Linux); on a headless host with no keyring, pass `--token` or
set `RELATUM_TOKEN` instead. `--output json` switches output from human-readable text
to the server's JSON.

## License

The workspace is **AGPL-3.0-only**, except the `relatum-cli` crate, which is
**GPL-3.0-only**. The API definition and the generated OpenAPI spec are licensed either **MIT or 
Apache 2.0**, at your option. Licenses are declared in the crate manifests.
