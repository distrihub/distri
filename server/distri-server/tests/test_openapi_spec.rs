//! Verify the OpenAPI spec generates valid JSON.

use utoipa::OpenApi;

#[test]
fn generate_openapi_spec_is_valid_json() {
    let doc = distri_server::openapi::ServerApiDoc::openapi();
    let json = doc.to_json().expect("OpenAPI spec must serialize to JSON");
    // Validate it parses back
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("OpenAPI JSON must parse");
    // Must have paths and info
    assert!(parsed.get("info").is_some(), "spec must have info section");
    assert!(
        parsed.get("paths").is_some(),
        "spec must have paths section"
    );
}
