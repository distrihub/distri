//! Client-side HTTP request handler.
//!
//! Auto-detects whether to execute locally or proxy through the server:
//! - If all `$VAR_NAME` references are in local `env_vars` → execute locally
//! - If any are unresolved OR `x-connection-id` is present → proxy to `POST /request`

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use distri_types::http_request::{HttpMethod, HttpRequestInput, HttpRequestResponse};
use distri_types::resolve::{extract_vars, substitute_string};
use distri_types::ToolResponse;

use crate::{Distri, ExternalToolRegistry};

/// Execute an HTTP request, auto-detecting whether to run locally or proxy.
pub async fn execute_http_request(
    input: &HttpRequestInput,
    env_vars: &HashMap<String, String>,
    client: &Distri,
) -> Result<HttpRequestResponse> {
    // If URL is relative and REQUEST_BASE_URL is available, prepend it
    let mut input = input.clone();
    if input.url.starts_with('/') {
        if let Some(base) = env_vars.get("REQUEST_BASE_URL") {
            input.url = format!("{}{}", base.trim_end_matches('/'), input.url);
        }
    }

    // Collect all $VAR references from url, headers, body
    let mut all_vars = extract_vars(&input.url);
    for (k, v) in &input.headers {
        all_vars.extend(extract_vars(k));
        all_vars.extend(extract_vars(v));
    }
    if let Some(ref body) = input.body {
        let body_str = serde_json::to_string(body).unwrap_or_default();
        all_vars.extend(extract_vars(&body_str));
    }
    all_vars.sort();
    all_vars.dedup();

    let has_connection_id = input.headers.contains_key("x-connection-id");
    let unresolved: Vec<&String> = all_vars.iter().filter(|v| !env_vars.contains_key(*v)).collect();

    // Proxy to server if secrets needed or connection-id present
    if !unresolved.is_empty() || has_connection_id {
        let reason = if has_connection_id {
            "x-connection-id present".to_string()
        } else {
            format!("unresolved vars: {}", unresolved.iter().map(|v| format!("${}", v)).collect::<Vec<_>>().join(", "))
        };
        return client
            .proxy_request(&input)
            .await
            .map_err(|e| anyhow::anyhow!("proxy to server failed ({}): {}", reason, e));
    }

    execute_locally(&input, env_vars).await
}

/// Execute the HTTP request locally after substituting env_vars.
async fn execute_locally(
    input: &HttpRequestInput,
    env_vars: &HashMap<String, String>,
) -> Result<HttpRequestResponse> {
    let url = substitute_string(&input.url, env_vars);
    let headers: HashMap<String, String> = input
        .headers
        .iter()
        .map(|(k, v)| (substitute_string(k, env_vars), substitute_string(v, env_vars)))
        .collect();
    let body = input.body.as_ref().map(|b| {
        distri_types::resolve::substitute_value(b, env_vars)
    });

    // Build reqwest request
    let http = reqwest::Client::new();
    let mut req = match input.method {
        HttpMethod::GET => http.get(&url),
        HttpMethod::POST => http.post(&url),
        HttpMethod::PUT => http.put(&url),
        HttpMethod::PATCH => http.patch(&url),
        HttpMethod::DELETE => http.delete(&url),
    };

    // Set headers
    let mut has_content_type = false;
    for (key, value) in &headers {
        if let (Ok(name), Ok(hval)) = (
            reqwest::header::HeaderName::from_bytes(key.to_lowercase().as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            if name == reqwest::header::CONTENT_TYPE {
                has_content_type = true;
            }
            req = req.header(name, hval);
        }
    }

    // Set body
    let method_str = input.method.to_string();
    if let Some(ref body) = body {
        if method_str != "GET" && method_str != "DELETE" {
            if !has_content_type {
                req = req.json(body);
            } else {
                let body_str = match body {
                    serde_json::Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                req = req.body(body_str);
            }
        }
    }

    let response = req
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("{} {} failed: {}", method_str, url, e))?;

    // Parse response
    let status = response.status().as_u16();
    let useful_headers: &[&str] = &[
        "content-type", "content-length", "location", "retry-after",
        "x-request-id", "x-ratelimit-limit", "x-ratelimit-remaining",
        "x-ratelimit-reset", "www-authenticate", "link",
    ];
    let response_headers: HashMap<String, String> = response
        .headers()
        .iter()
        .filter(|(k, _)| useful_headers.contains(&k.as_str()))
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let content_type = response_headers
        .get("content-type")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    let text = response.text().await.unwrap_or_default();

    let body = if content_type.contains("application/json") {
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::Value::String(text))
    } else if text.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(text)
    };

    Ok(HttpRequestResponse {
        status,
        ok: (200..300).contains(&status),
        headers: response_headers,
        body,
    })
}

/// Register a client-side `http_request` handler on the `ExternalToolRegistry`.
///
/// Intercepts `http_request` tool calls for all agents. Executes locally when
/// possible, proxies through the server when secrets or connection tokens are needed.
pub fn register_client_http_request(
    registry: &ExternalToolRegistry,
    client: Arc<Distri>,
    initial_env_vars: HashMap<String, String>,
) {
    let env_vars = Arc::new(initial_env_vars);

    registry.register("*", "http_request", move |call, _event| {
        let client = client.clone();
        let env_vars = env_vars.clone();
        async move {
            let input: HttpRequestInput = serde_json::from_value(call.input.clone())
                .map_err(|e| anyhow::anyhow!(
                    "http_request: invalid input: {}. Got: {}",
                    e,
                    serde_json::to_string(&call.input).unwrap_or_default()
                ))?;
            let result = execute_http_request(&input, &env_vars, &client).await
                .map_err(|e| anyhow::anyhow!(
                    "http_request {} {} failed: {}",
                    input.method,
                    input.url,
                    e
                ))?;
            Ok(ToolResponse::direct(
                call.tool_call_id.clone(),
                call.tool_name.clone(),
                serde_json::to_value(&result)?,
            ))
        }
    });
}
