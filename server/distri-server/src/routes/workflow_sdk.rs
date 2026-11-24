//! `POST /v1/workflows/sdk/call`: a sandboxed workflow `code` step's call to its run
//! (`@distri/runtime`, distrihub/distri-cloud#304 phase 3).
//!
//! The only credential is the step's **run token** (`Authorization: Bearer dsr_…`), minted when
//! the step started and revoked when it ended. A user token or an API key is not accepted here,
//! and a missing, unknown, expired or revoked run token is `401`. The call runs as the run
//! (`AgentOrchestrator::script_sdk_call`). The answer is `{ok: true, result}`, or
//! `{ok: false, error, code}` with `401` (`unauthorized`), `422` (`invalid`) or `200` (`failed`:
//! the call ran and failed).

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_types::script_sdk::{ScriptCall, ScriptCallError, RUN_TOKEN_PREFIX};
use serde_json::json;

/// The run token of a request: `Authorization: Bearer dsr_…`.
pub fn run_token(req: &HttpRequest) -> Option<String> {
    let value = req.headers().get("authorization")?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    token
        .starts_with(RUN_TOKEN_PREFIX)
        .then(|| token.to_string())
}

/// The response for a call's outcome.
pub fn sdk_response(result: Result<serde_json::Value, ScriptCallError>) -> HttpResponse {
    match result {
        Ok(v) => HttpResponse::Ok().json(json!({"ok": true, "result": v})),
        Err(e) => {
            let status = actix_web::http::StatusCode::from_u16(e.http_status())
                .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
            HttpResponse::build(status).json(json!({
                "ok": false,
                "error": e.to_string(),
                "code": e.code(),
            }))
        }
    }
}

pub async fn workflow_sdk_call(
    req: HttpRequest,
    body: web::Bytes,
    orchestrator: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let Some(token) = run_token(&req) else {
        return sdk_response(Err(ScriptCallError::Unauthorized));
    };
    // Authorize before reading the body, so a stranger learns nothing about the call shape.
    if orchestrator.script_sdk.get(&token).is_none() {
        return sdk_response(Err(ScriptCallError::Unauthorized));
    }
    let call: ScriptCall = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => return sdk_response(Err(ScriptCallError::Invalid(e.to_string()))),
    };
    sdk_response(orchestrator.script_sdk_call(&token, call).await)
}
