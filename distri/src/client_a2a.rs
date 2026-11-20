//! Typed A2A JSON-RPC calls on [`Distri`]: the agent card, `message/send`,
//! `message/stream`, `tasks/get|cancel|resubscribe`,
//! `tasks/pushNotificationConfig/*`, and the durable-runs extension's
//! pending calls. Used by `distri a2a …` in distri-cli.
//!
//! Streaming calls hand the caller each raw JSON-RPC frame (a
//! `serde_json::Value`) so a CLI can print the wire as-is or summarise it.

use distri_a2a::{
    AgentCard, JsonRpcRequest, MessageSendParams, PushNotificationConfig, SendMessageResult, Task,
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

/// A push notification config bound to a task
/// (`TaskPushNotificationConfig` in the A2A spec).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPushNotificationConfig {
    pub task_id: String,
    pub push_notification_config: PushConfig,
}

/// `PushNotificationConfig` with `Clone` (the distri-a2a one is not).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
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

    /// The agent card as served (`GET /agents/{agent}/.well-known/agent.json`),
    /// unparsed. `None` on 404.
    pub async fn get_agent_card_json(&self, agent: &str) -> Result<Option<Value>, ClientError> {
        let url = format!("{}/.well-known/agent.json", self.agent_url(agent));
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
        let rpc = a2a_request(method, params);
        let resp = self
            .http
            .post(self.agent_url(agent))
            .json(&rpc)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let frame: Value = serde_json::from_str(&text).map_err(|_| {
            ClientError::InvalidResponse(format!("{method} failed (status {status}): {text}"))
        })?;
        rpc_result(&frame)
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

    /// One streaming A2A JSON-RPC call (`message/stream`, `tasks/resubscribe`):
    /// `on_frame` gets every SSE `data:` frame as JSON, error frames included.
    pub async fn a2a_stream_json<F>(
        &self,
        agent: &str,
        method: &str,
        params: Value,
        mut on_frame: F,
    ) -> Result<(), ClientError>
    where
        F: FnMut(Value),
    {
        let rpc = a2a_request(method, params);
        let resp = self
            .http
            .post(self.agent_url(agent))
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
        Ok(())
    }

    /// `agent/authenticatedExtendedCard`.
    pub async fn get_authenticated_extended_card(&self, agent: &str) -> Result<Value, ClientError> {
        self.a2a_call_json(agent, "agent/authenticatedExtendedCard", json!({}))
            .await
    }

    /// `message/send` (blocking unless `configuration.blocking` is false).
    pub async fn send_message(
        &self,
        agent: &str,
        params: &MessageSendParams,
    ) -> Result<SendMessageResult, ClientError> {
        self.a2a_call(agent, "message/send", serde_json::to_value(params)?)
            .await
    }

    /// `tasks/get`, with an optional `historyLength`.
    pub async fn get_task(
        &self,
        agent: &str,
        task_id: &str,
        history_length: Option<u32>,
    ) -> Result<Task, ClientError> {
        self.a2a_call(
            agent,
            "tasks/get",
            task_query_params(task_id, history_length),
        )
        .await
    }

    /// `tasks/pushNotificationConfig/set`.
    pub async fn set_task_push_config(
        &self,
        agent: &str,
        task_id: &str,
        config: PushConfig,
    ) -> Result<TaskPushNotificationConfig, ClientError> {
        let params = TaskPushNotificationConfig {
            task_id: task_id.to_string(),
            push_notification_config: config,
        };
        self.a2a_call(
            agent,
            "tasks/pushNotificationConfig/set",
            serde_json::to_value(params)?,
        )
        .await
    }

    /// `tasks/pushNotificationConfig/get`.
    pub async fn get_task_push_config(
        &self,
        agent: &str,
        task_id: &str,
        config_id: Option<&str>,
    ) -> Result<TaskPushNotificationConfig, ClientError> {
        let mut params = json!({ "id": task_id });
        if let Some(c) = config_id {
            params["pushNotificationConfigId"] = json!(c);
        }
        self.a2a_call(agent, "tasks/pushNotificationConfig/get", params)
            .await
    }

    /// `tasks/pushNotificationConfig/list`.
    pub async fn list_task_push_configs(
        &self,
        agent: &str,
        task_id: &str,
    ) -> Result<Vec<TaskPushNotificationConfig>, ClientError> {
        self.a2a_call(
            agent,
            "tasks/pushNotificationConfig/list",
            json!({ "id": task_id }),
        )
        .await
    }

    /// `tasks/pushNotificationConfig/delete`.
    pub async fn delete_task_push_config(
        &self,
        agent: &str,
        task_id: &str,
        config_id: &str,
    ) -> Result<(), ClientError> {
        self.a2a_call_json(
            agent,
            "tasks/pushNotificationConfig/delete",
            json!({ "id": task_id, "pushNotificationConfigId": config_id }),
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

    /// Tasks via the REST list (`GET /tasks?thread_id&status&limit`); A2A has
    /// no `tasks/list` on this server. `status` is distri's task status
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

/// `{ "id": task_id, "historyLength": n }` for `tasks/get` / `tasks/resubscribe`.
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
