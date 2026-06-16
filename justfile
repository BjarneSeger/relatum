# relatum dev tasks. Run `just` to list.

# Departments offered by both the server (authority) and web (dropdown).
departments := "blue,red"
# API base URL the web frontend talks to.
api_url := "http://localhost:8080"
# Vendored htmx version (see `update-htmx`).
htmx_version := "2.0.9"

# List available recipes.
default:
    @just --list

# Run the full dev stack: API (mock auth) + SSR web frontend.
# Ctrl-C stops both. Log in with `tok-ins`, `tok-tr`, `tok-sig`, `tok-out`.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    RELATUM_SSO_BACKEND=mock RELATUM_DIRECTORY_BACKEND=mock RELATUM_DEPARTMENTS={{departments}} \
        cargo run -p relatum-server --features dev &
    api_pid=$!
    trap 'kill "$api_pid" 2>/dev/null || true' EXIT
    RELATUM_WEB_DEPARTMENTS={{departments}} \
        cargo run -p relatum-web -- --api-url {{api_url}}

# Run only the API server (mock SSO + mock users) on :8080.
api:
    RELATUM_SSO_BACKEND=mock RELATUM_DIRECTORY_BACKEND=mock RELATUM_DEPARTMENTS={{departments}} \
        cargo run -p relatum-server --features dev

# Run only the web frontend on :8081 (expects API at {{api_url}}).
web:
    RELATUM_WEB_DEPARTMENTS={{departments}} \
        cargo run -p relatum-web -- --api-url {{api_url}}

# Download a fresh vendored htmx and repoint the embed. Rebuild after.
# Usage: `just update-htmx` or `just update-htmx 2.0.10`.
update-htmx version=htmx_version:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="crates/relatum-web/static"
    meta="crates/relatum-web/src/handlers/meta.rs"
    dest="$dir/htmx-{{version}}.min.js"
    echo "downloading htmx {{version}} -> $dest"
    curl -fsSL "https://unpkg.com/htmx.org@{{version}}/dist/htmx.min.js" -o "$dest"
    # Drop older vendored copies so the binary only embeds one.
    for f in "$dir"/htmx-*.min.js; do
        [ "$f" = "$dest" ] || { echo "removing old $f"; rm -f "$f"; }
    done
    # Repoint the include_str! path + doc comment in meta.rs.
    sed -i -E \
        -e "s#htmx-[0-9]+\.[0-9]+\.[0-9]+\.min\.js#htmx-{{version}}.min.js#g" \
        -e "s#htmx [0-9]+\.[0-9]+\.[0-9]+#htmx {{version}}#g" \
        "$meta"
    echo "done. rebuild to embed: cargo build -p relatum-web"
