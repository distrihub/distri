//! HTTP request execution with variable resolution.
//!
//! Provides `execute_http_request` which resolves `$VAR_NAME` references,
//! handles `x-connection-id` OAuth injection, and executes the request.
//!
//! Used by the `POST /request` server route.

use std::collections::HashMap;

use distri_types::http_request::{HttpRequestInput, HttpRequestResponse, HttpMethod};

use crate::tools::resolve::{
    extract_vars, extract_vars_from_value, resolve_all, resolve_connection_token,
    substitute_string, ResolveContext,
};

/// Useful response headers to include (skip noise like cache-control, x-powered-by).
const USEFUL_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "location",
    "retry-after",
    "x-request-id",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "www-authenticate",
    "link",
];

/// Execute an HTTP request with variable resolution.
///
/// Resolves `$VAR_NAME` references in url, headers, and body from the
/// provided `ResolveContext`. Handles `x-connection-id` for OAuth Bearer
/// token injection.
pub async fn execute_http_request(
    input: &HttpRequestInput,
    resolve_ctx: &ResolveContext,
) -> Result<HttpRequestResponse, anyhow::Error> {
    // 1. Check for x-connection-id (consumed, not forwarded)
    let connection_id = input.headers.get("x-connection-id").cloned();

    // 2. Collect all $VAR references
    let mut all_vars = extract_vars(&input.url);
    for (k, v) in &input.headers {
        all_vars.extend(extract_vars(k));
        all_vars.extend(extract_vars(v));
    }
    if let Some(ref body) = input.body {
        all_vars.extend(extract_vars_from_value(body));
    }
    all_vars.sort();
    all_vars.dedup();

    // 3. Resolve variables
    let resolved = resolve_all(&all_vars, resolve_ctx)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // 4. Substitute resolved values
    let url = substitute_string(&input.url, &resolved);
    let headers: HashMap<String, String> = input
        .headers
        .iter()
        .filter(|(k, _)| k.as_str() != "x-connection-id")
        .map(|(k, v)| (substitute_string(k, &resolved), substitute_string(v, &resolved)))
        .collect();
    let body = input
        .body
        .as_ref()
        .map(|b| distri_types::resolve::substitute_value(b, &resolved));

    // 5. Build request headers
    let mut header_map = reqwest::header::HeaderMap::new();
    let mut has_content_type = false;

    for (key, value) in &headers {
        if let (Ok(name), Ok(hval)) = (
            reqwest::header::HeaderName::from_bytes(key.to_lowercase().as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            if name == reqwest::header::CONTENT_TYPE {
                has_content_type = true;
            }
            header_map.insert(name, hval);
        }
    }

    // 6. If x-connection-id was present, resolve and inject Bearer token
    if let Some(ref conn_id) = connection_id {
        let (_provider, access_token) = resolve_connection_token(conn_id, resolve_ctx)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        header_map.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", access_token).parse().unwrap(),
        );
    }

    // 7. Build and send request
    let client = reqwest::Client::new();
    let method_str = input.method.to_string();
    let mut request = match input.method {
        HttpMethod::GET => client.get(&url),
        HttpMethod::POST => client.post(&url),
        HttpMethod::PUT => client.put(&url),
        HttpMethod::PATCH => client.patch(&url),
        HttpMethod::DELETE => client.delete(&url),
    };

    request = request.headers(header_map);

    if let Some(ref body) = body {
        if method_str != "GET" && method_str != "DELETE" {
            if !has_content_type {
                request = request.json(body);
            } else {
                let body_str = match body {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                request = request.body(body_str);
            }
        }
    }

    let response = request
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await?;

    // 8. Read response
    let status = response.status().as_u16();
    let response_headers: HashMap<String, String> = response
        .headers()
        .iter()
        .filter(|(k, _)| USEFUL_HEADERS.contains(&k.as_str()))
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let content_type = response_headers
        .get("content-type")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let response_text = response.text().await.unwrap_or_default();

    let body = if content_type.contains("application/json") {
        serde_json::from_str(&response_text)
            .unwrap_or_else(|_| serde_json::Value::String(response_text))
    } else if response_text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(response_text)
    };

    Ok(HttpRequestResponse {
        status,
        ok: (200..300).contains(&status),
        headers: response_headers,
        body,
    })
}
