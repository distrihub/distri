//! Typed A2A 1.0 JSON-RPC calls on [`Distri`]: the agent card,
//! `SendMessage`, `SendStreamingMessage`, `GetTask` / `ListTasks` /
//! `CancelTask` / `SubscribeToTask`, the push-config methods, and the
//! durable-runs extension's pending calls. Used by `distri tasks …`,
//! `distri agents card` and `distri run --json`.
//!
//! Every call may activate A2A extensions (`A2A-Extensions` request header);
//! the server echoes the ones it activated in the same response header,
//! returned alongside the result.
//!
//! Streaming calls hand the caller each raw JSON-RPC frame (a
//! `serde_json::Value`) so a CLI can print the wire as-is or summarise it.

use distri_a2a::{
    AgentCard, JsonRpcRequest, ListTasksResponse, PushNotificationConfig, SendMessageRequest,
    SendMessageResponse, Task, methods,
};
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ClientError, Distri};

/// A JSON-RPC error answered by the server (`error` in the response).
#[derive(Debug, Clone)]
pub struct A2aRpcError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for A2aRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (JSON-RPC {})", self.message, self.code)
    }
}

/// A push notification config bound to a task (`TaskPushNotificationConfig`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskPushNotificationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub push_notification_config: PushConfig,
}

/// `PushNotificationConfig` (url, id, token).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl From<PushNotificationConfig> for PushConfig {
    fn from(c: PushNotificationConfig) -> Self {
        Self {
            id: c.id,
            url: c.url,
            token: c.token,
        }
    }
}

/// The extensions the server activated (`A2A-Extensions` response header).
pub fn activated_extensions(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    distri_a2a::parse_extensions_header(
        headers
            .get_all(distri_a2a::EXTENSIONS_HEADER)
            .iter()
            .filter_map(|v| v.to_str().ok()),
    )
}

fn with_extensions(req: reqwest::RequestBuilder, extensions: &[String]) -> reqwest::RequestBuilder {
    let req = req.header(distri_a2a::VERSION_HEADER, distri_a2a::A2A_VERSION);
    if extensions.is_empty() {
        req
    } else {
        req.header(distri_a2a::EXTENSIONS_HEADER, extensions.join(", "))
    }
}

/// Build a JSON-RPC 2.0 request with a fresh id.
pub fn a2a_request(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(Value::String(uuid::Uuid::new_v4().to_string())),
        method: method.to_string(),
        params,
    }
}

/// Split a JSON-RPC response into its result or an error.
pub fn rpc_result(frame: &Value) -> Result<Value, ClientError> {
    if let Some(err) = frame.get("error").filter(|e| !e.is_null()) {
        let e = A2aRpcError {
            code: err.get("code").and_then(Value::as_i64).unwrap_or(0) as i32,
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
        };
        return Err(ClientError::InvalidResponse(e.to_string()));
    }
    Ok(frame.get("result").cloned().unwrap_or(Value::Null))
}

impl Distri {
    fn agent_url(&self, agent: &str) -> String {
        format!("{}/agents/{}", self.base_url.trim_end_matches('/'), agent)
    }

    /// The agent card as served (`GET /agents/{agent}/.well-known/agent-card.json`),
    /// unparsed. `None` on 404.
    pub async fn get_agent_card_json(&self, agent: &str) -> Result<Option<Value>, ClientError> {
        let url = format!("{}/.well-known/agent-card.json", self.agent_url(agent));
        let resp = self.http.get(url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "failed to fetch agent card {agent} (status {status}): {text}"
            )));
        }
        Ok(Some(resp.json().await?))
    }

    /// The agent card, typed. `None` on 404.
    pub async fn get_agent_card(&self, agent: &str) -> Result<Option<AgentCard>, ClientError> {
        match self.get_agent_card_json(agent).await? {
            Some(v) => Ok(Some(serde_json::from_value(v)?)),
            None => Ok(None),
        }
    }

    /// One A2A JSON-RPC call (non-streaming); returns the raw `result`.
    pub async fn a2a_call_json(
        &self,
        agent: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, ClientError> {
        self.a2a_call_json_ext(agent, method, params, &[])
            .await
            .map(|(v, _)| v)
    }

    /// One A2A JSON-RPC call activating `extensions`; returns the raw
    /// `result` and the extensions the server activated.
    pub async fn a2a_call_json_ext(
        &self,
        agent: &str,
        method: &str,
        params: Value,
        extensions: &[String],
    ) -> Result<(Value, Vec<String>), ClientError> {
        let rpc = a2a_request(method, params);
        let resp = with_extensions(self.http.post(self.agent_url(agent)), extensions)
            .json(&rpc)
            .send()
            .await?;
        let status = resp.status();
        let activated = activated_extensions(resp.headers());
        let text = resp.text().await.unwrap_or_default();
        let frame: Value = serde_json::from_str(&text).map_err(|_| {
            ClientError::InvalidResponse(format!("{method} failed (status {status}): {text}"))
        })?;
        Ok((rpc_result(&frame)?, activated))
    }

    /// One A2A JSON-RPC call (non-streaming), decoding `result` into `T`.
    pub async fn a2a_call<T: DeserializeOwned>(
        &self,
        agent: &str,
        method: &str,
        params: Value,
    ) -> Result<T, ClientError> {
        let v = self.a2a_call_json(agent, method, params).await?;
        Ok(serde_json::from_value(v)?)
    }

    /// One streaming A2A JSON-RPC call (`SendStreamingMessage`,
    /// `SubscribeToTask`): `on_frame` gets every SSE `data:` frame as JSON,
    /// error frames included.
    pub async fn a2a_stream_json<F>(
        &self,
        agent: &str,
        method: &str,
        params: Value,
        on_frame: F,
    ) -> Result<(), ClientError>
    where
        F: FnMut(Value),
    {
        self.a2a_stream_json_ext(agent, method, params, &[], on_frame)
            .await
            .map(|_| ())
    }

    /// [`Self::a2a_stream_json`] activating `extensions`; returns the
    /// extensions the server activated.
    pub async fn a2a_stream_json_ext<F>(
        &self,
        agent: &str,
        method: &str,
        params: Value,
        extensions: &[String],
        mut on_frame: F,
    ) -> Result<Vec<String>, ClientError>
    where
        F: FnMut(Value),
    {
        let rpc = a2a_request(method, params);
        let resp = with_extensions(self.http.post(self.agent_url(agent)), extensions)
            .header("Accept", "text/event-stream")
            .json(&rpc)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "{method} failed (status {status}): {text}"
            )));
        }
        let activated = activated_extensions(resp.headers());
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(pos) = buf.find("\n\n") {
                let block: String = buf.drain(..pos + 2).collect();
                if let Some(frame) = sse_block_json(&block) {
                    on_frame(frame);
                }
            }
        }
        if let Some(frame) = sse_block_json(&buf) {
            on_frame(frame);
        }
        Ok(activated)
    }

    /// `GetExtendedAgentCard`.
    pub async fn get_authenticated_extended_card(&self, agent: &str) -> Result<Value, ClientError> {
        self.a2a_call_json(agent, methods::GET_EXTENDED_AGENT_CARD, json!({}))
            .await
    }

    /// `SendMessage` (blocking unless `configuration.blocking` is false).
    pub async fn send_message(
        &self,
        agent: &str,
        params: &SendMessageRequest,
    ) -> Result<SendMessageResponse, ClientError> {
        self.a2a_call(agent, methods::SEND_MESSAGE, serde_json::to_value(params)?)
            .await
    }

    /// `GetTask`, with an optional `historyLength`.
    pub async fn get_task(
        &self,
        agent: &str,
        task_id: &str,
        history_length: Option<u32>,
    ) -> Result<Task, ClientError> {
        self.a2a_call(
            agent,
            methods::GET_TASK,
            task_query_params(task_id, history_length),
        )
        .await
    }

    /// `ListTasks`.
    pub async fn list_tasks_a2a(
        &self,
        agent: &str,
        params: &distri_a2a::ListTasksRequest,
    ) -> Result<ListTasksResponse, ClientError> {
        self.a2a_call(agent, methods::LIST_TASKS, serde_json::to_value(params)?)
            .await
    }

    /// `CreateTaskPushNotificationConfig`.
    pub async fn set_task_push_config(
        &self,
        agent: &str,
        task_id: &str,
        config: PushConfig,
    ) -> Result<TaskPushNotificationConfig, ClientError> {
        let params = json!({
            "taskId": task_id,
            "configId": config.id,
            "config": config,
        });
        self.a2a_call(agent, methods::CREATE_PUSH_CONFIG, params)
            .await
    }

    /// `GetTaskPushNotificationConfig`.
    pub async fn get_task_push_config(
        &self,
        agent: &str,
        task_id: &str,
        config_id: Option<&str>,
    ) -> Result<TaskPushNotificationConfig, ClientError> {
        let params = json!({ "taskId": task_id, "id": config_id.unwrap_or_default() });
        self.a2a_call(agent, methods::GET_PUSH_CONFIG, params).await
    }

    /// `ListTaskPushNotificationConfigs`.
    pub async fn list_task_push_configs(
        &self,
        agent: &str,
        task_id: &str,
    ) -> Result<Vec<TaskPushNotificationConfig>, ClientError> {
        #[derive(Deserialize)]
        struct Configs {
            #[serde(default)]
            configs: Vec<TaskPushNotificationConfig>,
        }
        let r: Configs = self
            .a2a_call(
                agent,
                methods::LIST_PUSH_CONFIGS,
                json!({ "taskId": task_id }),
            )
            .await?;
        Ok(r.configs)
    }

    /// `DeleteTaskPushNotificationConfig`.
    pub async fn delete_task_push_config(
        &self,
        agent: &str,
        task_id: &str,
        config_id: &str,
    ) -> Result<(), ClientError> {
        self.a2a_call_json(
            agent,
            methods::DELETE_PUSH_CONFIG,
            json!({ "taskId": task_id, "id": config_id }),
        )
        .await
        .map(|_| ())
    }

    /// Durable-runs extension: the task's pending calls
    /// (`GET /tasks/{task_id}/pending-calls`). `None` on 404 (unknown task, or
    /// a server without durable runs).
    pub async fn list_task_pending_calls(
        &self,
        task_id: &str,
    ) -> Result<Option<Vec<Value>>, ClientError> {
        let url = format!(
            "{}/tasks/{}/pending-calls",
            self.base_url.trim_end_matches('/'),
            task_id
        );
        let resp = self.http.get(url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Self::json_or_error_a2a(resp, "list pending calls")
            .await
            .map(Some)
    }

    /// Tasks via the REST list (`GET /tasks?thread_id&status&limit`). `status` is distri's task status
    /// (`pending`, `running`, `input_required`, `completed`, `failed`,
    /// `canceled`).
    pub async fn list_tasks_filtered(
        &self,
        thread_id: Option<&str>,
        status: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<distri_types::Task>, ClientError> {
        let url = format!("{}/tasks", self.base_url.trim_end_matches('/'));
        let mut q: Vec<(&str, String)> = Vec::new();
        if let Some(t) = thread_id {
            q.push(("thread_id", t.to_string()));
        }
        if let Some(s) = status {
            q.push(("status", s.to_string()));
        }
        if let Some(l) = limit {
            q.push(("limit", l.to_string()));
        }
        let resp = self.http.get(url).query(&q).send().await?;
        Self::json_or_error_a2a(resp, "list tasks").await
    }

    async fn json_or_error_a2a<T: DeserializeOwned>(
        resp: reqwest::Response,
        what: &str,
    ) -> Result<T, ClientError> {
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ClientError::InvalidResponse(format!(
                "{what} failed (status {status}): {text}"
            )));
        }
        Ok(resp.json().await?)
    }
}

/// `{ "id": task_id, "historyLength": n }` for `GetTask`.
pub fn task_query_params(task_id: &str, history_length: Option<u32>) -> Value {
    let mut v = json!({ "id": task_id });
    if let Some(n) = history_length {
        v["historyLength"] = json!(n);
    }
    v
}

/// The JSON of one SSE block's `data:` lines, if any.
fn sse_block_json(block: &str) -> Option<Value> {
    let data: Vec<&str> = block
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim_start)
        .collect();
    if data.is_empty() {
        return None;
    }
    let joined = data.join("\n");
    if joined.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&joined).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_result_splits_error_and_result() {
        let ok = json!({"jsonrpc":"2.0","id":"1","result":{"id":"t"}});
        assert_eq!(rpc_result(&ok).unwrap(), json!({"id":"t"}));
        let err = json!({"jsonrpc":"2.0","id":"1","error":{"code":-32601,"message":"Method not found: x"}});
        let e = rpc_result(&err).unwrap_err().to_string();
        assert!(
            e.contains("Method not found: x") && e.contains("-32601"),
            "{e}"
        );
    }

    #[test]
    fn sse_blocks_parse_data_lines() {
        assert_eq!(
            sse_block_json("event: message\ndata: {\"a\":1}\n\n"),
            Some(json!({"a":1}))
        );
        assert_eq!(sse_block_json(": keepalive\n\n"), None);
        assert_eq!(sse_block_json("data: \n\n"), None);
    }

    #[test]
    fn task_params_carry_history_length() {
        assert_eq!(task_query_params("t", None), json!({"id":"t"}));
        assert_eq!(
            task_query_params("t", Some(2)),
            json!({"id":"t","historyLength":2})
        );
    }
}
