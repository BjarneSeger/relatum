//! Generate the HTTP client from `relatum-api`'s live OpenAPI document.
//!
//! The spec comes from [`relatum_api::openapi_json`], which derives it from the
//! actual served routes — so any change to a route or DTO regenerates the client on
//! the next build. The generated module is written to `OUT_DIR/codegen.rs` and
//! pulled into the crate via `include!` in `lib.rs`.

use std::{env, fs, path::Path};

fn main() {
    let spec_json = relatum_api::openapi_json().expect("serialize OpenAPI document");

    let mut spec: openapiv3::OpenAPI =
        serde_json::from_str(&spec_json).expect("parse OpenAPI document");

    // utoipa emits an OpenAPI 3.1 document, but progenitor's parser (`openapiv3`)
    // only accepts 3.0.x and rejects the "3.1.0" version string. Our schemas use no
    // 3.1-only constructs (no `type: [..., "null"]`, no `examples` arrays that
    // openapiv3 can't model — those would already have failed the deserialize
    // above), so it is safe to relabel the document as 3.0 for generation. If a
    // future DTO introduces a genuinely 3.1-only shape, the `from_str` above will
    // fail loudly rather than silently mis-generate.
    spec.openapi = "3.0.3".to_string();

    let mut generator = progenitor::Generator::default();
    let tokens = generator
        .generate_tokens(&spec)
        .expect("generate client tokens");
    let ast = syn::parse2(tokens).expect("parse generated tokens");
    let content = prettyplease::unparse(&ast);

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let out_file = Path::new(&out_dir).join("codegen.rs");
    fs::write(&out_file, content).expect("write generated client");

    // The generated code only depends on the spec, which is baked into relatum-api;
    // rebuild whenever this script changes.
    println!("cargo:rerun-if-changed=build.rs");
}
