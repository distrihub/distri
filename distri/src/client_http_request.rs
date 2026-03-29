//! Client-side HTTP request handler that mirrors the server-side `HttpRequestTool`.
//!
//! Resolves `$VAR_NAME` references via `SecretCache` and handles `x-connection-id`
//! for OAuth Bearer token injection.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use distri_types::resolve::{
    extract_vars, extract_vars_from_value, substitute_string, substitute_value,
};
use distri_types::ToolResponse;
use serde_json::{json, Value};

use crate::secret_cache::SecretCache;
use crate::{Distri, ExternalToolRegistry};

/// Execute an HTTP request with variable resolution and connection token support.
///
/// Input/output format matches the server-side `HttpRequestTool`:
/// - Input: `{ url, method, headers, body }`
/// - Output: `{ status, ok, headers, body }`
pub async fn execute_http_request(
    input: &Value,
    secret_cache: &SecretCache,
    env_vars: &HashMap<String, String>,
) -> Result<Value> {
    // 1. Parse input
    let method = input
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_uppercase();

    let raw_url = input
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?
        .to_string();

    let headers_value = input.get("headers").cloned().unwrap_or_else(|| json!({}));
    let body_value = input.get("body").cloned();

    // 2. Check for x-connection-id
    let connection_id = headers_value
        .as_object()
        .and_then(|h| h.get("x-connection-id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 3. Collect all $VAR references
    let mut all_vars = extract_vars(&raw_url);
    all_vars.extend(extract_vars_from_value(&headers_value));
    if let Some(ref body) = body_value {
        all_vars.extend(extract_vars_from_value(body));
    }
    all_vars.sort();
    all_vars.dedup();

    // 4. Resolve variables via SecretCache
    let resolved = secret_cache.resolve_vars(&all_vars, env_vars).await?;

    // 5. Substitute resolved values
    let url = substitute_string(&raw_url, &resolved);
    let headers_value = substitute_value(&headers_value, &resolved);
    let body_value = body_value.map(|b| substitute_value(&b, &resolved));

    // 6. Build request headers
    let mut header_map = reqwest::header::HeaderMap::new();
    let mut has_content_type = false;

    if let Some(headers_obj) = headers_value.as_object() {
        for (key, value) in headers_obj {
            if key == "x-connection-id" {
                continue; // consumed, not forwarded
            }
            if let Some(val) = value.as_str() {
                if let (Ok(name), Ok(hval)) = (
                    reqwest::header::HeaderName::from_bytes(key.to_lowercase().as_bytes()),
                    reqwest::header::HeaderValue::from_str(val),
                ) {
                    if name == reqwest::header::CONTENT_TYPE {
                        has_content_type = true;
                    }
                    header_map.insert(name, hval);
                }
            }
        }
    }

    // 7. If connection_id present, resolve token and inject Bearer header
    if let Some(ref conn_id) = connection_id {
        let access_token = secret_cache.resolve_connection_token(conn_id).await?;
        header_map.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", access_token).parse()?,
        );
    }

    // 8. Build and send request
    let client = reqwest::Client::new();
    let mut request = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        other => anyhow::bail!("Unsupported HTTP method: {}", other),
    };

    request = request.headers(header_map);

    if let Some(body) = &body_value {
        if method != "GET" && method != "DELETE" {
            if !has_content_type {
                request = request.json(body);
            } else {
                let body_str = match body {
                    Value::String(s) => s.clone(),
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

    // 9. Read response
    let status = response.status().as_u16();

    let useful_headers: &[&str] = &[
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

    let response_text = response.text().await.unwrap_or_default();

    let is_json = content_type.contains("application/json");
    let response_body: Value = if is_json {
        serde_json::from_str(&response_text).unwrap_or_else(|_| Value::String(response_text))
    } else if response_text.is_empty() {
        Value::Null
    } else {
        Value::String(response_text)
    };

    let headers_json: Value = serde_json::to_value(&response_headers).unwrap_or(json!({}));

    // 10. Build result — scrub secrets from the response body
    let secret_values: Vec<&str> = resolved.values().map(|v| v.as_str()).collect();
    let result = json!({
        "status": status,
        "ok": (200..300).contains(&status),
        "headers": headers_json,
        "body": scrub_secrets(&response_body, &secret_values),
    });

    Ok(result)
}

/// Remove secret values from a JSON value to prevent leaking them in tool output.
fn scrub_secrets(value: &Value, secrets: &[&str]) -> Value {
    if secrets.is_empty() {
        return value.clone();
    }
    match value {
        Value::String(s) => {
            let mut result = s.clone();
            for secret in secrets {
                if !secret.is_empty() {
                    result = result.replace(secret, "***");
                }
            }
            Value::String(result)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| scrub_secrets(v, secrets)).collect())
        }
        Value::Object(map) => {
            let new_map = map
                .iter()
                .map(|(k, v)| (k.clone(), scrub_secrets(v, secrets)))
                .collect();
            Value::Object(new_map)
        }
        other => other.clone(),
    }
}

/// Register a client-side `http_request` handler on the `ExternalToolRegistry`.
///
/// This intercepts `http_request` tool calls for all agents (`"*"`) and executes
/// them locally with secret resolution via the Distri cloud API.
pub fn register_client_http_request(
    registry: &ExternalToolRegistry,
    client: Arc<Distri>,
    initial_env_vars: HashMap<String, String>,
) {
    let secret_cache = Arc::new(SecretCache::new(client));
    let env_vars = Arc::new(initial_env_vars);

    registry.register("*", "http_request", move |call, _event| {
        let secret_cache = secret_cache.clone();
        let env_vars = env_vars.clone();
        async move {
            let result = execute_http_request(&call.input, &secret_cache, &env_vars).await?;
            Ok(ToolResponse::direct(
                call.tool_call_id.clone(),
                call.tool_name.clone(),
                result,
            ))
        }
    });
}
