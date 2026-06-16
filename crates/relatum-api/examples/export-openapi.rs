//! Emit the OpenAPI document to `openapi.json`.
//!
//! The spec is derived from the **actual routes** (see [`relatum_api::routes`]) via
//! [`relatum_api::openapi_json`], so it stays in lockstep with the handlers. The
//! same helper is consumed by `relatum-client`'s build script, so the generated
//! client cannot drift from the served routes either.
//!
//! ```sh
//! cargo run -p relatum-api --example export-openapi
//! # → writes ./openapi.json
//!
//! # then, e.g. a TypeScript client (requires openapi-generator, a Java tool):
//! openapi-generator-cli generate -i openapi.json -g typescript-fetch -o clients/ts
//! ```

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = relatum_api::openapi_json()?;

    // Allow an optional output path argument; default to ./openapi.json.
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("openapi.json"));

    std::fs::write(&out, json)?;
    println!("wrote OpenAPI spec to {}", out.display());
    Ok(())
}
