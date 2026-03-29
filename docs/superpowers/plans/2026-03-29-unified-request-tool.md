# Unified Request Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify `request`, `api_request`, and `connection_request` into a single `request` tool with `$VAR` resolution from context env vars, connection tokens, and workspace secrets.

**Architecture:** One shared `resolve_variables()` function resolves `$VAR_NAME` references from three sources (priority: context env vars > connection tokens > workspace secrets). `RequestTool` uses it for inline HTTP substitution. `InjectConnectionEnvTool` uses it to populate executor env vars for shell/browsr. Both read `TokenFetcher` from `ExecutorContext`.

**Tech Stack:** Rust, reqwest, wiremock (new dev-dependency for mock HTTP), distri-core tools framework

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `server/distri-core/src/tools/resolve.rs` | **Create** | Shared `resolve_variables()` + `$VAR` extraction |
| `server/distri-core/src/tools/request.rs` | **Rewrite** | Unified request tool using `resolve_variables()` |
| `server/distri-core/src/tools/inject_env.rs` | **Modify** | Use shared `resolve_variables()`, read `TokenFetcher` from context |
| `server/distri-core/src/tools/mod.rs` | **Modify** | Add `resolve` module, update cast function |
| `server/distri-core/src/agent/context.rs` | **Modify** | Add `token_fetcher: Option<TokenFetcher>` to `ExecutorContext` |
| `server/distri-core/src/tools/builtin.rs` | **Modify** | Add `InjectConnectionEnvTool` to builtin tools |
| `server/distri-core/src/tools/simulator.rs` | **Modify** | Clean up tool names |
| `server/distri-core/Cargo.toml` | **Modify** | Add `wiremock` dev-dependency |
| `server/distri-core/src/tests/request_tool.rs` | **Create** | Unit tests for request tool |
| `server/distri-core/src/tests/mod.rs` | **Modify** | Add `request_tool` test module |
| `distri/src/api_request_tool.rs` | **Delete** | Replaced by server-side `request` |
| `distri/src/lib.rs` | **Modify** | Remove `api_request_tool` module and exports |
| `distri-cli/src/tools.rs` | **Modify** | Remove `register_api_request_handler` |
| `distri/src/client.rs` | **Modify** | Remove `connection_request()` and `ConnectionProxyResponse` |
| `distri/src/renderers/mod.rs` | **Modify** | Add `request` renderer |
| `server/agents/skills/distri_platform.md` | **Modify** | Remove `connection_request`, update examples |

---

### Task 1: Add `wiremock` dev-dependency

**Files:**
- Modify: `server/distri-core/Cargo.toml`

- [ ] **Step 1: Add wiremock to dev-dependencies**

In `server/distri-core/Cargo.toml`, add under `[dev-dependencies]`:

```toml
wiremock = "0.6"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p distri-core`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add server/distri-core/Cargo.toml
git commit -m "chore: add wiremock dev-dependency for request tool tests"
```

---

### Task 2: Create shared `resolve_variables()` function

**Files:**
- Create: `server/distri-core/src/tools/resolve.rs`
- Modify: `server/distri-core/src/tools/mod.rs`
- Create: `server/distri-core/src/tests/request_tool.rs`
- Modify: `server/distri-core/src/tests/mod.rs`

- [ ] **Step 1: Add `resolve` module to `mod.rs`**

In `server/distri-core/src/tools/mod.rs`, add after the `pub mod tool_search;` line:

```rust
pub mod resolve;
```

- [ ] **Step 2: Add test module**

In `server/distri-core/src/tests/mod.rs`, add:

```rust
mod request_tool;
```

- [ ] **Step 3: Write failing tests for variable extraction and resolution**

Create `server/distri-core/src/tests/request_tool.rs`:

```rust
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::tools::resolve::{extract_vars, resolve_all, ResolveContext};
use distri_types::stores::SecretRecord;

/// Helper: create a ResolveContext with given env vars, secrets, and token fetcher
fn make_resolve_ctx(
    env_vars: HashMap<String, String>,
    secrets: Vec<(&str, &str)>,
    token_fetcher: Option<crate::tools::inject_env::TokenFetcher>,
) -> ResolveContext {
    let secret_store = Arc::new(InMemorySecretStore::new(secrets));
    ResolveContext {
        env_vars,
        secret_store: Some(secret_store as Arc<dyn distri_types::SecretStore>),
        token_fetcher,
    }
}

/// Minimal in-memory SecretStore for tests
struct InMemorySecretStore {
    secrets: HashMap<String, String>,
}

impl InMemorySecretStore {
    fn new(entries: Vec<(&str, &str)>) -> Self {
        Self {
            secrets: entries.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }
}

#[async_trait::async_trait]
impl distri_types::SecretStore for InMemorySecretStore {
    async fn list(&self) -> anyhow::Result<Vec<SecretRecord>> {
        Ok(self.secrets.iter().map(|(k, v)| SecretRecord {
            id: k.clone(),
            key: k.clone(),
            value: v.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }).collect())
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<SecretRecord>> {
        Ok(self.secrets.get(key).map(|v| SecretRecord {
            id: key.to_string(),
            key: key.to_string(),
            value: v.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }))
    }

    async fn create(&self, _secret: distri_types::stores::NewSecret) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }
    async fn update(&self, _key: &str, _value: &str) -> anyhow::Result<SecretRecord> {
        unimplemented!()
    }
    async fn delete(&self, _key: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

#[test]
fn test_extract_vars_from_string() {
    let vars = extract_vars("Bearer $API_KEY");
    assert_eq!(vars, vec!["API_KEY"]);
}

#[test]
fn test_extract_vars_multiple() {
    let vars = extract_vars("$HOST/api?token=$TOKEN");
    assert!(vars.contains(&"HOST".to_string()));
    assert!(vars.contains(&"TOKEN".to_string()));
    assert_eq!(vars.len(), 2);
}

#[test]
fn test_extract_vars_none() {
    let vars = extract_vars("https://example.com/api");
    assert!(vars.is_empty());
}

#[test]
fn test_extract_vars_in_json_value() {
    let value = serde_json::json!({
        "url": "https://$HOST/api",
        "headers": {
            "Authorization": "Bearer $TOKEN",
            "x-org": "$ORG_ID"
        },
        "body": {
            "nested": {
                "key": "$SECRET"
            }
        }
    });
    let vars = crate::tools::resolve::extract_vars_from_value(&value);
    assert!(vars.contains(&"HOST".to_string()));
    assert!(vars.contains(&"TOKEN".to_string()));
    assert!(vars.contains(&"ORG_ID".to_string()));
    assert!(vars.contains(&"SECRET".to_string()));
    assert_eq!(vars.len(), 4);
}

#[tokio::test]
async fn test_resolve_from_env_vars() {
    let mut env = HashMap::new();
    env.insert("API_KEY".to_string(), "sk-123".to_string());
    let ctx = make_resolve_ctx(env, vec![], None);

    let resolved = resolve_all(&["API_KEY".to_string()], &ctx).await.unwrap();
    assert_eq!(resolved.get("API_KEY").unwrap(), "sk-123");
}

#[tokio::test]
async fn test_resolve_from_secrets() {
    let ctx = make_resolve_ctx(HashMap::new(), vec![("DB_PASS", "secret123")], None);

    let resolved = resolve_all(&["DB_PASS".to_string()], &ctx).await.unwrap();
    assert_eq!(resolved.get("DB_PASS").unwrap(), "secret123");
}

#[tokio::test]
async fn test_resolve_env_takes_priority_over_secrets() {
    let mut env = HashMap::new();
    env.insert("KEY".to_string(), "from_env".to_string());
    let ctx = make_resolve_ctx(env, vec![("KEY", "from_secret")], None);

    let resolved = resolve_all(&["KEY".to_string()], &ctx).await.unwrap();
    assert_eq!(resolved.get("KEY").unwrap(), "from_env");
}

#[tokio::test]
async fn test_resolve_unresolved_var_errors() {
    let ctx = make_resolve_ctx(HashMap::new(), vec![], None);

    let result = resolve_all(&["MISSING_VAR".to_string()], &ctx).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("MISSING_VAR"), "Error should name the variable: {}", err);
}

#[tokio::test]
async fn test_resolve_connection_token() {
    let fetcher: crate::tools::inject_env::TokenFetcher = Arc::new(|conn_id: String| -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send>> {
        Box::pin(async move {
            if conn_id == "google_123" {
                Ok(("google".to_string(), "ya29.token_xyz".to_string()))
            } else {
                Err(format!("Unknown connection: {}", conn_id))
            }
        })
    });

    let ctx = make_resolve_ctx(HashMap::new(), vec![], Some(fetcher));

    let resolved = crate::tools::resolve::resolve_connection_token("google_123", &ctx).await.unwrap();
    assert_eq!(resolved.0, "google");
    assert_eq!(resolved.1, "ya29.token_xyz");
}

#[test]
fn test_substitute_vars_in_string() {
    let mut resolved = HashMap::new();
    resolved.insert("API_KEY".to_string(), "sk-123".to_string());
    resolved.insert("HOST".to_string(), "api.example.com".to_string());

    let result = crate::tools::resolve::substitute_string("https://$HOST/v1?key=$API_KEY", &resolved);
    assert_eq!(result, "https://api.example.com/v1?key=sk-123");
}

#[test]
fn test_substitute_vars_in_value() {
    let mut resolved = HashMap::new();
    resolved.insert("TOKEN".to_string(), "abc".to_string());

    let value = serde_json::json!({
        "headers": { "Authorization": "Bearer $TOKEN" },
        "plain": "no vars here"
    });

    let result = crate::tools::resolve::substitute_value(&value, &resolved);
    assert_eq!(result["headers"]["Authorization"], "Bearer abc");
    assert_eq!(result["plain"], "no vars here");
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p distri-core request_tool -- --nocapture 2>&1 | head -30`
Expected: compilation errors (resolve module doesn't exist yet)

- [ ] **Step 5: Implement `resolve.rs`**

Create `server/distri-core/src/tools/resolve.rs`:

```rust
//! Shared variable resolution for request and inject_connection_env tools.
//!
//! Resolves `$VAR_NAME` references from three sources (in priority order):
//! 1. Context env vars (highest priority)
//! 2. Connection tokens (via TokenFetcher callback)
//! 3. Workspace secrets (from SecretStore)

use std::collections::HashMap;
use std::sync::Arc;

use distri_types::SecretStore;
use regex::Regex;

use super::inject_env::TokenFetcher;

/// Everything needed to resolve `$VAR` references.
pub struct ResolveContext {
    pub env_vars: HashMap<String, String>,
    pub secret_store: Option<Arc<dyn SecretStore>>,
    pub token_fetcher: Option<TokenFetcher>,
}

/// Extract all `$VAR_NAME` references from a string.
/// Variable names must be `[A-Z0-9_]+`.
pub fn extract_vars(s: &str) -> Vec<String> {
    let re = Regex::new(r"\$([A-Z][A-Z0-9_]*)").unwrap();
    re.captures_iter(s)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// Recursively extract all `$VAR_NAME` references from a JSON value (string fields only).
pub fn extract_vars_from_value(value: &serde_json::Value) -> Vec<String> {
    let mut vars = Vec::new();
    collect_vars_from_value(value, &mut vars);
    vars.sort();
    vars.dedup();
    vars
}

fn collect_vars_from_value(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.extend(extract_vars(s)),
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_vars_from_value(v, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_vars_from_value(v, out);
            }
        }
        _ => {}
    }
}

/// Resolve a list of variable names from the context.
/// Returns a map of var_name -> resolved_value.
/// Errors if any variable is unresolved.
pub async fn resolve_all(
    var_names: &[String],
    ctx: &ResolveContext,
) -> Result<HashMap<String, String>, String> {
    let mut resolved = HashMap::new();

    for name in var_names {
        // 1. Check env vars (highest priority)
        if let Some(val) = ctx.env_vars.get(name) {
            resolved.insert(name.clone(), val.clone());
            continue;
        }

        // 2. Check workspace secrets
        if let Some(ref store) = ctx.secret_store {
            if let Ok(Some(secret)) = store.get(name).await {
                resolved.insert(name.clone(), secret.value);
                continue;
            }
        }

        // 3. Not found
        return Err(format!("unresolved variable: ${}", name));
    }

    Ok(resolved)
}

/// Fetch a connection token via the TokenFetcher callback.
/// Returns (provider_name, access_token).
pub async fn resolve_connection_token(
    connection_id: &str,
    ctx: &ResolveContext,
) -> Result<(String, String), String> {
    let fetcher = ctx
        .token_fetcher
        .as_ref()
        .ok_or_else(|| "no token fetcher configured — cannot resolve connection".to_string())?;
    (fetcher)(connection_id.to_string()).await
}

/// Replace all `$VAR_NAME` occurrences in a string with resolved values.
pub fn substitute_string(s: &str, resolved: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (name, value) in resolved {
        result = result.replace(&format!("${}", name), value);
    }
    result
}

/// Recursively substitute `$VAR_NAME` in all string values of a JSON value.
pub fn substitute_value(
    value: &serde_json::Value,
    resolved: &HashMap<String, String>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(substitute_string(s, resolved)),
        serde_json::Value::Object(map) => {
            let new_map: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), substitute_value(v, resolved)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| substitute_value(v, resolved)).collect())
        }
        other => other.clone(),
    }
}
```

- [ ] **Step 6: Add `regex` dependency if not present**

Check if `regex` is already in `distri-core` dependencies. If not, add it to `server/distri-core/Cargo.toml`:

```toml
regex = { workspace = true }
```

Or if not in workspace:

```toml
regex = "1"
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p distri-core request_tool -- --nocapture`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add server/distri-core/src/tools/resolve.rs server/distri-core/src/tools/mod.rs server/distri-core/src/tests/request_tool.rs server/distri-core/src/tests/mod.rs server/distri-core/Cargo.toml
git commit -m "feat: add shared resolve_variables for request tool"
```

---

### Task 3: Add `token_fetcher` to `ExecutorContext`

**Files:**
- Modify: `server/distri-core/src/agent/context.rs`

- [ ] **Step 1: Add field to ExecutorContext**

In `server/distri-core/src/agent/context.rs`, add after the `pub env_vars` field (line 158):

```rust
    /// Token fetcher callback for resolving connection OAuth tokens.
    /// Used by request tool and inject_connection_env to fetch tokens on demand.
    pub token_fetcher: Option<crate::tools::inject_env::TokenFetcher>,
```

- [ ] **Step 2: Initialize to None in all constructors**

Find all places where `ExecutorContext` is constructed (search for `ExecutorContext {` in `context.rs`). Add `token_fetcher: None` to each. Also update `new_task()` and `continue_as()` to propagate the parent's `token_fetcher`:

In `new_task()` and `continue_as()`, add:

```rust
token_fetcher: self.token_fetcher.clone(),
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p distri-core`
Expected: compiles (all constructors updated)

- [ ] **Step 4: Commit**

```bash
git add server/distri-core/src/agent/context.rs
git commit -m "feat: add token_fetcher to ExecutorContext"
```

---

### Task 4: Rewrite `RequestTool` with variable resolution

**Files:**
- Rewrite: `server/distri-core/src/tools/request.rs`
- Modify: `server/distri-core/src/tests/request_tool.rs` (add integration tests)

- [ ] **Step 1: Write failing integration tests for the request tool**

Append to `server/distri-core/src/tests/request_tool.rs`:

```rust
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{method, path, header};
use crate::tools::ExecutorContextTool;
use crate::tools::request::RequestTool;
use crate::agent::ExecutorContext;
use tokio::sync::RwLock;

/// Helper: create a minimal ExecutorContext with given env_vars and optional secret store + token fetcher
async fn make_executor_context(
    env_vars: HashMap<String, String>,
    secrets: Vec<(&str, &str)>,
    token_fetcher: Option<crate::tools::inject_env::TokenFetcher>,
) -> Arc<ExecutorContext> {
    let stores = crate::tests::test_store_config().await;
    // Inject secrets into the store if provided
    if let Some(ref secret_store) = stores.secret_store {
        for (k, v) in &secrets {
            let _ = secret_store.create(distri_types::stores::NewSecret {
                key: k.to_string(),
                value: v.to_string(),
            }).await;
        }
    }

    let mut ctx = ExecutorContext::new_minimal_for_test(stores);
    *ctx.env_vars.write().await = env_vars;
    ctx.token_fetcher = token_fetcher;
    Arc::new(ctx)
}

#[tokio::test]
async fn test_request_tool_success_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": [1, 2, 3]})))
        .mount(&server)
        .await;

    let mut env = HashMap::new();
    env.insert("BASE_URL".to_string(), server.uri());
    let ctx = make_executor_context(env, vec![], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc1".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": "$BASE_URL/api/items",
            "method": "GET"
        }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    assert_eq!(parts.len(), 1);
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 200);
        assert_eq!(data["ok"], true);
        assert_eq!(data["data"]["items"], serde_json::json!([1, 2, 3]));
    } else {
        panic!("Expected Part::Data");
    }
}

#[tokio::test]
async fn test_request_tool_error_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/items"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "title is required"})))
        .mount(&server)
        .await;

    let mut env = HashMap::new();
    env.insert("BASE_URL".to_string(), server.uri());
    let ctx = make_executor_context(env, vec![], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc2".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": "$BASE_URL/api/items",
            "method": "POST",
            "body": {"title": ""}
        }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 400);
        assert_eq!(data["ok"], false);
        assert_eq!(data["error"], "title is required");
    } else {
        panic!("Expected Part::Data");
    }
}

#[tokio::test]
async fn test_request_tool_non_json_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/error"))
        .respond_with(ResponseTemplate::new(500).set_body_string("<html>Internal Server Error</html>"))
        .mount(&server)
        .await;

    let mut env = HashMap::new();
    env.insert("URL".to_string(), format!("{}/error", server.uri()));
    let ctx = make_executor_context(env, vec![], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc3".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({ "url": "$URL", "method": "GET" }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 500);
        assert_eq!(data["ok"], false);
        // Non-JSON body preserved as string
        assert!(data["error"].as_str().unwrap().contains("Internal Server Error"));
    } else {
        panic!("Expected Part::Data");
    }
}

#[tokio::test]
async fn test_request_tool_unresolved_var_errors() {
    let ctx = make_executor_context(HashMap::new(), vec![], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc4".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": "https://example.com",
            "method": "GET",
            "headers": { "Authorization": "Bearer $MISSING_TOKEN" }
        }),
    };

    let result = tool.execute_with_executor_context(call, ctx).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("MISSING_TOKEN"), "Should mention the missing var: {}", err);
}

#[tokio::test]
async fn test_request_tool_secret_resolution() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secure"))
        .and(header("Authorization", "Bearer secret_key_123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let mut env = HashMap::new();
    env.insert("BASE".to_string(), server.uri());
    let ctx = make_executor_context(env, vec![("API_KEY", "secret_key_123")], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc5".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": "$BASE/secure",
            "method": "GET",
            "headers": { "Authorization": "Bearer $API_KEY" }
        }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 200);
        assert_eq!(data["ok"], true);
    } else {
        panic!("Expected Part::Data");
    }
}

#[tokio::test]
async fn test_request_tool_env_var_priority_over_secret() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/check"))
        .and(header("x-key", "env_value"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"source": "env"})))
        .mount(&server)
        .await;

    let mut env = HashMap::new();
    env.insert("BASE".to_string(), server.uri());
    env.insert("KEY".to_string(), "env_value".to_string());
    // Secret has same name but lower priority
    let ctx = make_executor_context(env, vec![("KEY", "secret_value")], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc6".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": "$BASE/check",
            "method": "GET",
            "headers": { "x-key": "$KEY" }
        }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 200);
    } else {
        panic!("Expected Part::Data");
    }
}

#[tokio::test]
async fn test_request_tool_connection_id_injects_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sheets"))
        .and(header("Authorization", "Bearer ya29.google_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"sheets": []})))
        .mount(&server)
        .await;

    let fetcher: crate::tools::inject_env::TokenFetcher = Arc::new(|conn_id| {
        Box::pin(async move {
            if conn_id == "google_123" {
                Ok(("google".to_string(), "ya29.google_token".to_string()))
            } else {
                Err(format!("Unknown connection: {}", conn_id))
            }
        })
    });

    let mut env = HashMap::new();
    env.insert("BASE".to_string(), server.uri());
    let ctx = make_executor_context(env, vec![], Some(fetcher)).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc7".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": "$BASE/sheets",
            "method": "GET",
            "headers": { "x-connection-id": "google_123" }
        }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 200);
        assert_eq!(data["ok"], true);
    } else {
        panic!("Expected Part::Data");
    }
}

#[tokio::test]
async fn test_request_tool_no_vars_plain_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})))
        .mount(&server)
        .await;

    let ctx = make_executor_context(HashMap::new(), vec![], None).await;

    let tool = RequestTool;
    let call = crate::types::ToolCall {
        tool_call_id: "tc8".to_string(),
        tool_name: "request".to_string(),
        input: serde_json::json!({
            "url": format!("{}/health", server.uri()),
            "method": "GET"
        }),
    };

    let parts = tool.execute_with_executor_context(call, ctx).await.unwrap();
    if let distri_types::Part::Data(data) = &parts[0] {
        assert_eq!(data["status"], 200);
        assert_eq!(data["ok"], true);
    } else {
        panic!("Expected Part::Data");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p distri-core request_tool -- --nocapture 2>&1 | head -20`
Expected: compilation errors (RequestTool doesn't have new implementation, `new_minimal_for_test` doesn't exist)

- [ ] **Step 3: Add `new_minimal_for_test` to ExecutorContext**

In `server/distri-core/src/agent/context.rs`, add a `#[cfg(test)]` helper:

```rust
#[cfg(test)]
impl ExecutorContext {
    /// Minimal context for unit testing tools in isolation.
    pub fn new_minimal_for_test(stores: distri_stores::InitializedStores) -> Self {
        use tokio::sync::mpsc;
        Self {
            thread_id: uuid::Uuid::new_v4().to_string(),
            task_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            agent_id: "test-agent".to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            user_id: "test-user".to_string(),
            identifier_id: None,
            workspace_id: None,
            channel_id: None,
            tenant_context: distri_types::TenantContext::default(),
            browser_session_id: None,
            additional_attributes: None,
            tools: Arc::new(RwLock::new(Vec::new())),
            orchestrator: None,
            stores: Some(stores),
            verbose: false,
            tool_metadata: None,
            event_tx: None,
            usage: Arc::new(RwLock::new(super::types::ContextUsage::default())),
            current_plan: Arc::new(RwLock::new(None)),
            task_status: Arc::new(RwLock::new(None)),
            final_result: Arc::new(RwLock::new(None)),
            current_step_id: Arc::new(RwLock::new(None)),
            current_message_id: Arc::new(RwLock::new(None)),
            env_vars: Arc::new(RwLock::new(std::collections::HashMap::new())),
            parent_tx: None,
            parent_task_id: None,
            dynamic_tools: None,
            hook_prompt_state: Arc::new(RwLock::new(Default::default())),
            hook_registry: Arc::new(RwLock::new(None)),
            default_model_settings: None,
            dry_run: false,
            token_fetcher: None,
        }
    }
}
```

- [ ] **Step 4: Rewrite `request.rs`**

Replace `server/distri-core/src/tools/request.rs` entirely:

```rust
//! Unified HTTP request tool.
//!
//! Resolves `$VAR_NAME` references from:
//! 1. Context env vars (highest priority)
//! 2. Workspace secrets (from SecretStore)
//!
//! Supports `x-connection-id` header for OAuth token injection via TokenFetcher.

use std::sync::Arc;

use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};

use crate::tools::resolve::{
    self, extract_vars_from_value, resolve_all, resolve_connection_token, substitute_string,
    substitute_value, ResolveContext,
};
use crate::{agent::ExecutorContext, tools::ExecutorContextTool, types::ToolCall, AgentError};

#[derive(Debug)]
pub struct RequestTool;

#[async_trait::async_trait]
impl Tool for RequestTool {
    fn get_name(&self) -> String {
        "request".to_string()
    }

    fn get_description(&self) -> String {
        "Make an HTTP request. Variables ($VAR_NAME) in url, headers, and body are auto-resolved \
         from context env vars, workspace secrets, and connections. \
         Use x-connection-id header to auto-inject OAuth bearer tokens."
            .to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["url", "method"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Request URL. Supports $VAR_NAME substitution."
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "description": "HTTP method"
                },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Request headers. Use x-connection-id for OAuth. Supports $VAR_NAME."
                },
                "body": {
                    "description": "Request body (sent as JSON for POST/PUT/PATCH). Supports $VAR_NAME in string values."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!("RequestTool requires ExecutorContext"))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for RequestTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input = &tool_call.input;

        let method = input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let raw_url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::ToolExecution("Missing 'url' parameter".into()))?;

        let headers_val = input.get("headers").cloned().unwrap_or(json!({}));
        let body_val = input.get("body").cloned();

        // Extract x-connection-id before variable resolution
        let connection_id = headers_val
            .get("x-connection-id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Build resolve context from ExecutorContext
        let env_vars = context.env_vars.read().await.clone();
        let secret_store = context
            .stores
            .as_ref()
            .and_then(|s| s.secret_store.clone());
        let resolve_ctx = ResolveContext {
            env_vars,
            secret_store,
            token_fetcher: context.token_fetcher.clone(),
        };

        // Collect all $VAR references from url, headers, body
        let mut all_vars = resolve::extract_vars(raw_url);
        all_vars.extend(extract_vars_from_value(&headers_val));
        if let Some(ref body) = body_val {
            all_vars.extend(extract_vars_from_value(body));
        }
        all_vars.sort();
        all_vars.dedup();

        // Resolve all variables
        let resolved = resolve_all(&all_vars, &resolve_ctx)
            .await
            .map_err(|e| AgentError::ToolExecution(e))?;

        // Substitute in url, headers, body
        let url = substitute_string(raw_url, &resolved);
        let headers_resolved = substitute_value(&headers_val, &resolved);
        let body_resolved = body_val.map(|b| substitute_value(&b, &resolved));

        // Build outgoing headers, stripping x-connection-id
        let mut header_map = reqwest::header::HeaderMap::new();
        header_map.insert("Content-Type", "application/json".parse().unwrap());
        if let Some(obj) = headers_resolved.as_object() {
            for (key, value) in obj {
                if key == "x-connection-id" {
                    continue; // Don't forward this
                }
                if let Some(val) = value.as_str() {
                    if let (Ok(name), Ok(hval)) = (
                        key.parse::<reqwest::header::HeaderName>(),
                        val.parse::<reqwest::header::HeaderValue>(),
                    ) {
                        header_map.insert(name, hval);
                    }
                }
            }
        }

        // If x-connection-id present, fetch OAuth token and inject Authorization
        if let Some(conn_id) = &connection_id {
            let (_provider, access_token) = resolve_connection_token(conn_id, &resolve_ctx)
                .await
                .map_err(|e| AgentError::ToolExecution(e))?;
            header_map.insert(
                "Authorization",
                format!("Bearer {}", access_token).parse().unwrap(),
            );
        }

        // Build and execute request
        let client = reqwest::Client::new();
        let mut request = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            "DELETE" => client.delete(&url),
            _ => {
                return Err(AgentError::ToolExecution(format!(
                    "Unsupported method: {}",
                    method
                )))
            }
        };
        request = request.headers(header_map);

        if let Some(body) = body_resolved {
            if method != "GET" && method != "DELETE" {
                request = request.json(&body);
            }
        }

        let response = request
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| AgentError::ToolExecution(format!("HTTP request failed: {e}")))?;

        let status = response.status().as_u16();
        let response_text = response.text().await.unwrap_or_default();
        let response_body: Value = serde_json::from_str(&response_text).unwrap_or_else(|_| {
            if response_text.is_empty() {
                json!(null)
            } else {
                Value::String(response_text)
            }
        });

        // Build consistent result
        let result = if (200..300).contains(&status) {
            let data = response_body
                .get("data")
                .cloned()
                .unwrap_or(response_body);
            json!({ "status": status, "ok": true, "data": data })
        } else {
            let error = response_body
                .get("error")
                .cloned()
                .unwrap_or(response_body);
            json!({ "status": status, "ok": false, "error": error })
        };

        Ok(vec![Part::Data(result)])
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p distri-core request_tool -- --nocapture`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add server/distri-core/src/tools/request.rs server/distri-core/src/tests/request_tool.rs server/distri-core/src/agent/context.rs
git commit -m "feat: rewrite request tool with variable resolution and connection support"
```

---

### Task 5: Update `InjectConnectionEnvTool` to use shared resolution

**Files:**
- Modify: `server/distri-core/src/tools/inject_env.rs`
- Modify: `server/distri-core/src/tools/builtin.rs`
- Modify: `server/distri-core/src/tools/mod.rs` (cast function)

- [ ] **Step 1: Rewrite `inject_env.rs` to read `TokenFetcher` from context**

Replace `server/distri-core/src/tools/inject_env.rs`:

```rust
//! Injects connection tokens and secrets into executor env vars.
//!
//! Uses the same variable resolution as the request tool.
//! Tokens are injected into ExecutorContext.env_vars so browsr shell
//! sessions and child agents can access them via os.getenv().

use crate::agent::ExecutorContext;
use crate::tools::resolve::{ResolveContext, resolve_connection_token};
use crate::tools::ExecutorContextTool;
use crate::types::ToolCall;
use crate::AgentError;
use distri_types::{Part, Tool, ToolContext};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Callback type for fetching a connection token given a connection_id.
/// Returns (provider_name, access_token) or an error.
pub type TokenFetcher = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send>>
        + Send
        + Sync,
>;

/// Tool that fetches a connection token and injects it as an environment variable.
/// The token never appears in conversation messages — only in env_vars map.
/// Child agents (via new_task/continue_as) inherit the env vars automatically.
#[derive(Debug)]
pub struct InjectConnectionEnvTool;

#[async_trait::async_trait]
impl Tool for InjectConnectionEnvTool {
    fn get_name(&self) -> String {
        "inject_connection_env".to_string()
    }

    fn get_description(&self) -> String {
        "Fetch a connection token and inject it as an environment variable. \
         The token is silently added to the execution context — child agents \
         and shell sessions will receive it automatically."
            .to_string()
    }

    fn needs_executor_context(&self) -> bool {
        true
    }

    fn get_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "connection_id": {
                    "type": "string",
                    "description": "The connection ID to fetch the token for"
                },
                "env_var": {
                    "type": "string",
                    "description": "Override the environment variable name (default: <PROVIDER>_TOKEN)"
                }
            },
            "required": ["connection_id"]
        })
    }

    async fn execute(
        &self,
        _tool_call: distri_types::ToolCall,
        _context: Arc<ToolContext>,
    ) -> Result<Vec<Part>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "InjectConnectionEnvTool requires ExecutorContext"
        ))
    }
}

#[async_trait::async_trait]
impl ExecutorContextTool for InjectConnectionEnvTool {
    async fn execute_with_executor_context(
        &self,
        tool_call: ToolCall,
        context: Arc<ExecutorContext>,
    ) -> Result<Vec<Part>, AgentError> {
        let input = &tool_call.input;

        let connection_id = input
            .get("connection_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AgentError::ToolExecution("Missing 'connection_id' parameter".to_string())
            })?;

        // Build resolve context from ExecutorContext
        let env_vars = context.env_vars.read().await.clone();
        let secret_store = context
            .stores
            .as_ref()
            .and_then(|s| s.secret_store.clone());
        let resolve_ctx = ResolveContext {
            env_vars,
            secret_store,
            token_fetcher: context.token_fetcher.clone(),
        };

        // Fetch token via shared resolution
        let (provider, access_token) = resolve_connection_token(connection_id, &resolve_ctx)
            .await
            .map_err(|e| AgentError::ToolExecution(e))?;

        // Determine env var name
        let env_var_name = input
            .get("env_var")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}_TOKEN", provider.to_uppercase()));

        // Inject into context env_vars
        {
            let mut env_vars = context.env_vars.write().await;
            env_vars.insert(env_var_name.clone(), access_token);
        }

        tracing::info!(
            "[inject_connection_env] Injected {} for provider '{}' (connection: {})",
            env_var_name,
            provider,
            connection_id
        );

        Ok(vec![Part::Data(json!({
            "injected": true,
            "provider": provider,
            "env_var": env_var_name,
            "connection_id": connection_id,
        }))])
    }
}
```

- [ ] **Step 2: Add `InjectConnectionEnvTool` to builtin tools and cast function**

In `server/distri-core/src/tools/builtin.rs`, add to the `tools` vec in `get_builtin_tools()`:

```rust
Arc::new(crate::tools::inject_env::InjectConnectionEnvTool) as Arc<dyn Tool>,
```

In `server/distri-core/src/tools/mod.rs`, add to `cast_to_executor_context_tool` match:

```rust
"inject_connection_env" => Ok(Box::new(inject_env::InjectConnectionEnvTool)),
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p distri-core`
Expected: compiles

- [ ] **Step 4: Run all tests**

Run: `cargo test -p distri-core -- --nocapture`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add server/distri-core/src/tools/inject_env.rs server/distri-core/src/tools/builtin.rs server/distri-core/src/tools/mod.rs
git commit -m "refactor: inject_connection_env uses shared resolution, reads TokenFetcher from context"
```

---

### Task 6: Remove client-side `api_request` tool and proxy

**Files:**
- Delete: `distri/src/api_request_tool.rs`
- Modify: `distri/src/lib.rs`
- Modify: `distri/src/client.rs`
- Modify: `distri-cli/src/tools.rs`

- [ ] **Step 1: Remove `api_request_tool` module from `lib.rs`**

In `distri/src/lib.rs`, remove:
- The `mod api_request_tool;` line
- The `pub use api_request_tool::{ApiRequestTool, api_request_definition, execute_api_request};` line

- [ ] **Step 2: Delete `api_request_tool.rs`**

Delete `distri/src/api_request_tool.rs`.

- [ ] **Step 3: Remove `connection_request` and `ConnectionProxyResponse` from `client.rs`**

In `distri/src/client.rs`, remove:
- The `ConnectionProxyResponse` struct
- The `connection_request()` method

- [ ] **Step 4: Remove `register_api_request_handler` from CLI tools**

In `distri-cli/src/tools.rs`, remove the `register_api_request_handler` function entirely.

In `distri-cli/src/chat.rs` and `distri-cli/src/main.rs`, remove all calls to `register_api_request_handler`.

Also remove the import of `register_api_request_handler` from `distri-cli/src/main.rs` (the `use` statement at the top).

- [ ] **Step 5: Fix any remaining compilation errors**

Run: `cargo check -p distri -p distri-cli 2>&1`
Fix any remaining references to `api_request`, `execute_api_request`, `ApiRequestTool`, `connection_request`, etc.

- [ ] **Step 6: Run full workspace check**

Run: `cargo check`
Expected: compiles

- [ ] **Step 7: Commit**

```bash
git add -u distri/src/api_request_tool.rs distri/src/lib.rs distri/src/client.rs distri-cli/src/tools.rs distri-cli/src/chat.rs distri-cli/src/main.rs
git commit -m "refactor: remove api_request tool and connection proxy — use server-side request tool"
```

---

### Task 7: Update CLI renderer for `request` tool output

**Files:**
- Modify: `distri/src/renderers/mod.rs`

- [ ] **Step 1: Add request tool renderer**

In `distri/src/renderers/mod.rs`, add before the `// Default` match arm:

```rust
        // HTTP request tool
        "request" => render_request(result),
```

Add the `render_request` function before `render_artifact`:

```rust
fn render_request(result: &ToolResponse) {
    use distri_types::Part;
    for part in &result.parts {
        if let Part::Data(value) = part {
            let status = value.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
            let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

            if ok {
                let data = value.get("data").unwrap_or(value);
                let preview = serde_json::to_string(data).unwrap_or_default();
                let preview = if preview.len() > 120 {
                    format!("{}...", &preview[..120])
                } else {
                    preview
                };
                println!(
                    "{}{}{} {} — {}{}",
                    COLOR_GRAY, RESULT_PREFIX, status, "OK", preview, COLOR_RESET
                );
            } else {
                let error = value.get("error").unwrap_or(value);
                let preview = serde_json::to_string(error).unwrap_or_default();
                let preview = if preview.len() > 200 {
                    format!("{}...", &preview[..200])
                } else {
                    preview
                };
                println!(
                    "{}{}{} ERR — {}{}",
                    COLOR_GRAY, RESULT_PREFIX, status, preview, COLOR_RESET
                );
            }
            return;
        }
    }
    render_tool_result(result);
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p distri`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add distri/src/renderers/mod.rs
git commit -m "feat: add request tool CLI renderer"
```

---

### Task 8: Update simulator and skill files

**Files:**
- Modify: `server/distri-core/src/tools/simulator.rs`
- Modify: `server/agents/skills/distri_platform.md`

- [ ] **Step 1: Clean up simulator tool lists**

In `server/distri-core/src/tools/simulator.rs`, in `ALWAYS_SIMULATE`:
- Remove `"api_request"` and `"connection_request"`
- Keep `"request"` (add if not present)

- [ ] **Step 2: Update distri_platform skill**

Replace the Connections section in `server/agents/skills/distri_platform.md`:

```markdown
### Connections (OAuth Integrations)
| Action | Params | Description |
|--------|--------|-------------|
| `list_connections` | — | List connected services with scopes and capabilities |
| `connect` | `provider, scopes?, additional_scopes?` | Connect a provider or expand scopes |
| `get_connection_usage` | `connection_id` | Get API docs and examples for a connection |

To make authenticated API calls to connected services, use the `request` tool with `x-connection-id` header:

```
curl -X GET https://sheets.googleapis.com/v4/spreadsheets \
  -H "x-connection-id: <connection_id>"

curl -X POST https://gmail.googleapis.com/gmail/v1/users/me/messages/send \
  -H "x-connection-id: <connection_id>" \
  -d '{"raw": "<base64_encoded_message>"}'
```

Variables (`$VAR_NAME`) in url, headers, and body are auto-resolved from workspace secrets and context env vars.

```
curl -X GET https://api.example.com/v1/items \
  -H "Authorization: Bearer $API_KEY"
```
```

Remove the old `connection_request` row and the "Never use raw tokens" warning.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p distri-core`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add server/distri-core/src/tools/simulator.rs server/agents/skills/distri_platform.md
git commit -m "refactor: update simulator and skill docs for unified request tool"
```

---

### Task 9: Make `--verbose` global in CLI

**Files:**
- Already done in `distri-cli/src/main.rs` (from earlier in this conversation)

- [ ] **Step 1: Verify the `global = true` flag is set**

In `distri-cli/src/main.rs`, confirm the verbose flag has:

```rust
#[clap(long, short, global = true)]
verbose: bool,
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p distri-cli`
Expected: compiles

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 4: Commit all remaining changes**

```bash
git add -A
git commit -m "feat: unified request tool with $VAR resolution, connection support, and tests"
```
