# relatum-web Helm chart

Deploys the **relatum-web** frontend — a stateless, server-side-rendered Axum app
that renders the relatum UI over the HTTP API. It holds no database, sessions or
secrets of its own; the only thing it needs is a reachable `relatum-server`.

## Prerequisites

- Helm 3.8+ (OCI support)
- A pre-built image at `image.repository` (see the repo's `images.yml` workflow,
  which publishes `ghcr.io/bjarneseger/relatum-web`)
- A running `relatum-server` reachable at `config.apiUrl`

## Quick start

```sh
helm install web oci://ghcr.io/bjarneseger/charts/relatum-web \
  --version 0.1.0 \
  --set config.apiUrl=http://relatum-server \
  --set config.publicUrl=https://relatum.example
```

`config.publicUrl` is **required**: it builds the SSO `redirect_uri`
(`<publicUrl>/auth/callback`, which must be registered with your IdP) and decides
whether the session cookie is marked `Secure` (true when it starts with `https://`).

## Common values

| Key                    | Default                          | Description                                                        |
| ---------------------- | -------------------------------- | ------------------------------------------------------------------ |
| `image.repository`     | `ghcr.io/bjarneseger/relatum-web`| Container image.                                                   |
| `image.tag`            | `""`                             | Defaults to the chart `appVersion`.                                |
| `replicaCount`         | `1`                              | Safe to raise — the app is stateless.                              |
| `containerPort`        | `8081`                           | `RELATUM_WEB_LISTEN` is derived as `0.0.0.0:<this>`.               |
| `config.apiUrl`        | `http://relatum-server`          | Base URL of the relatum API (`RELATUM_WEB_API_URL`).               |
| `config.publicUrl`     | `""` (**required**)              | The frontend's externally-reachable base URL (`RELATUM_WEB_PUBLIC_URL`). |
| `config.departments`   | `[]`                             | Mirrors the server's department set (`RELATUM_WEB_DEPARTMENTS`).   |
| `ingress.enabled`      | `false`                          | Expose via an Ingress.                                             |
| `httproute.enabled`    | `false`                          | Expose via a Gateway API `HTTPRoute` (attach via `httproute.parentRefs`). |
| `autoscaling.enabled`  | `false`                          | CPU-based HPA (safe — stateless).                                 |
| `resources`            | `{}`                             | Pod resource requests/limits.                                      |

Health endpoint: `/healthz` (used for both liveness and readiness).

## Exposure: Ingress or Gateway API

Both off by default — enable at most one:

- `ingress.enabled=true` — classic `Ingress`.
- `httproute.enabled=true` — Gateway API `HTTPRoute` attached to an external
  Gateway via `httproute.parentRefs` (the chart ships no Gateway/GatewayClass;
  requires the Gateway API CRDs in the cluster). Set `httproute.hostnames` to the
  host in `config.publicUrl` so the SSO `redirect_uri` and the Secure-cookie
  decision line up with the hostname the Gateway actually serves:

  ```sh
  helm install web oci://ghcr.io/bjarneseger/charts/relatum-web \
    --set config.apiUrl=http://relatum-server \
    --set config.publicUrl=https://relatum.example \
    --set httproute.enabled=true \
    --set httproute.parentRefs[0].name=external-gateway \
    --set httproute.parentRefs[0].namespace=gateway-system \
    --set 'httproute.hostnames[0]=relatum.example'
  ```

  A cross-namespace Gateway also needs a `ReferenceGrant` in the Gateway's
  namespace allowing `HTTPRoute`s from this release's namespace.
