# relatum-server Helm chart

Deploys the `relatum-server` HTTP backend, with a Valkey session store (via the
upstream [valkey-io](https://github.com/valkey-io/valkey-helm) chart) and an
optional bundled Postgres.

The server is configured entirely through `RELATUM_*` environment variables; the
chart renders those from its values, keeping connection URLs that carry
credentials in Secrets rather than the ConfigMap.

## Prerequisites

- Helm 3.8+ (OCI support, used for the Valkey dependency)
- A built `relatum-server` image — none is published by the repo yet. Set
  `image.repository`/`image.tag` to your image.

```sh
helm dependency build deploy/helm/relatum-server
```

## Quick start (self-contained)

Bundles Postgres and Valkey so there are no external prerequisites:

```sh
helm install relatum deploy/helm/relatum-server \
  --set image.repository=ghcr.io/you/relatum-server \
  --set image.tag=0.1.0 \
  --set postgresql.enabled=true
```

> A bare `helm install` (no database source) **fails on purpose**: with
> `config.data.backend=postgres` you must pick where Postgres lives, rather than
> silently running with no persistence. Pick one of the four options below.

## Database (`RELATUM_DATA_URL`)

Resolved in this precedence, used when `config.data.backend=postgres`:

| Option | Set | Use when |
| --- | --- | --- |
| Existing secret | `database.existingSecret`, `database.existingSecretKey` (default `uri`) | A Postgres operator already provisions a connection-URL secret. **Recommended for operators.** |
| Inline URL | `database.url` | You have a full `postgres://…` URL. |
| Bundled Postgres | `postgresql.enabled=true` | Self-contained deploy; the chart runs a single-instance Postgres StatefulSet. |
| External parts | `externalDatabase.host` (+ `username`/`password`/`database`/`port`/`params`) | A managed Postgres reachable by host/port. |

With an operator-provided secret:

```sh
helm install relatum deploy/helm/relatum-server \
  --set database.existingSecret=relatum-pg-credentials \
  --set database.existingSecretKey=uri
```

The bundled Postgres password is generated on first install and **retained
across upgrades** via a `lookup` of the chart-managed secret; set
`postgresql.password` to pin it explicitly.

## Sessions (`RELATUM_SESSIONS_URL`)

Used when `config.sessions.backend=redis`. By default the bundled Valkey
subchart (`valkey.enabled=true`, auth off) is used and the URL is derived
automatically. To point at an external Valkey/Redis, set `sessions.url` or
`sessions.existingSecret`. If you enable `valkey.auth`, supply the credentialed
URL via `sessions.url`/`sessions.existingSecret` — the auto-derived URL assumes
no auth.

## Replicas and statefulness

relatum-server keeps **no** local state when backed by Postgres + Valkey, so a
`Deployment` is the right primitive and pods are interchangeable — scale via
`replicaCount` or `autoscaling`. Startup migrations are safe under concurrent
pods (sqlx takes a Postgres advisory lock).

The `memory` backends keep users, reports and sessions in-process and are **not**
replica-safe. The chart refuses `replicaCount > 1` (or autoscaling) while either
backend is `memory`.

## Exposure: Ingress or Gateway API

Two independent options, both off by default — enable at most one:

- `ingress.enabled=true` — classic `Ingress` (set `ingress.className`,
  `ingress.hosts`, `ingress.tls`).
- `httproute.enabled=true` — Gateway API `HTTPRoute`. The chart ships **no**
  Gateway or GatewayClass; point the route at an existing external Gateway via
  `httproute.parentRefs`. Each rule defaults its backend to this chart's Service,
  so the minimal config is just a parentRef:

  ```sh
  helm install relatum deploy/helm/relatum-server \
    --set httproute.enabled=true \
    --set httproute.parentRefs[0].name=external-gateway \
    --set httproute.parentRefs[0].namespace=gateway-system \
    --set 'httproute.hostnames[0]=relatum.example'
  ```

  Requires the Gateway API CRDs installed in the cluster. A cross-namespace
  Gateway also needs a `ReferenceGrant` in the Gateway's namespace allowing
  `HTTPRoute`s from this release's namespace.

## Common values

| Key | Default | Description |
| --- | --- | --- |
| `image.repository` / `image.tag` | `ghcr.io/bjarneseger/relatum-server` / chart appVersion | Server image. |
| `replicaCount` | `1` | Replicas (requires external backends if > 1). |
| `config.data.backend` | `postgres` | `postgres` or `memory`. |
| `config.sessions.backend` | `redis` | `redis` (Valkey) or `memory`. |
| `config.sessions.ttlSecs` | `86400` | Session lifetime. |
| `containerPort` | `8080` | Listen port; `RELATUM_LISTEN` is derived from it. |
| `logLevel` | `info` | `RUST_LOG` value. |
| `postgresql.enabled` | `false` | Bundle a single-instance Postgres. |
| `valkey.enabled` | `true` | Bundle the Valkey subchart. |
| `ingress.enabled` | `false` | Expose via Ingress. |
| `httproute.enabled` | `false` | Expose via a Gateway API `HTTPRoute` (attach to an external Gateway via `httproute.parentRefs`). |

See [`values.yaml`](./values.yaml) for the full set, and `helm show values
oci://ghcr.io/valkey-io/valkey-helm/valkey` for the Valkey subchart options
(passed under the `valkey:` key).
