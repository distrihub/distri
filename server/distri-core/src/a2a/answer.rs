//! Answering `TASK_STATE_INPUT_REQUIRED` in plain A2A (G2).
//!
//! A `SendMessage` / `SendStreamingMessage` whose `message.taskId` names a
//! task waiting on client tool calls answers those calls:
//!
//! - each `DataPart` with `data.tool_call_id` (tagged
//!   `distri/part_type: tool_result`, the 0.x `{part_type, data}` wrapper, or
//!   a plain DataPart) completes that call with `data.result` (or the
//!   tool-result `parts`);
//! - a text-only message completes the one pending call with
//!   `{"text": "<the text>"}` (several pending: `-32602`).
//!
//! Completion goes through [`AgentOrchestrator::complete_tool_call`] (first
//! writer wins), in durable and non-durable mode alike.

use distri_a2a::{Part as A2aPart, PartContent};
use distri_types::a2a_extensions::PART_TYPE_KEY;
use distri_types::{Part, ToolCall, ToolResponse};
use serde_json::{json, Value};

use crate::AgentOrchestrator;

/// A client tool call a task waits on.
#[derive(Debug, Clone, PartialEq)]
pub struct WaitingCall {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: Value,
}

impl WaitingCall {
    pub fn tool_call(&self) -> ToolCall {
        ToolCall {
            tool_call_id: self.tool_call_id.clone(),
            tool_name: self.tool_name.clone(),
            input: self.input.clone(),
        }
    }

    /// `{tool_call_id, tool_name, input}` — the tool_call DataPart's body.
    pub fn json(&self) -> Value {
        json!({
            "tool_call_id": self.tool_call_id,
            "tool_name": self.tool_name,
            "input": self.input,
        })
    }
}

/// What a message to a waiting task says.
#[derive(Debug, Clone, PartialEq)]
pub enum AnswerParts {
    /// Tool results, one per answered call (`tool_name` may be empty).
    Results(Vec<ToolResponse>),
    /// Text only.
    Text(String),
    /// Neither: not an answer.
    None,
}

/// Read a message's parts as an answer.
pub fn answer_parts(parts: &[A2aPart]) -> AnswerParts {
    let mut results = Vec::new();
    for part in parts {
        if let Some(r) = tool_result_of(part) {
            results.push(r);
        }
    }
    if !results.is_empty() {
        return AnswerParts::Results(results);
    }
    let texts: Option<Vec<&str>> = parts.iter().map(|p| p.as_text()).collect();
    match texts {
        Some(t) if !t.is_empty() => {
            let text = t.join("\n");
            if text.trim().is_empty() {
                AnswerParts::None
            } else {
                AnswerParts::Text(text)
            }
        }
        _ => AnswerParts::None,
    }
}

/// A tool result carried by a DataPart, if the part is one.
fn tool_result_of(part: &A2aPart) -> Option<ToolResponse> {
    let PartContent::Data(data) = &part.content else {
        return None;
    };
    let tag = part
        .metadata
        .as_ref()
        .and_then(|m| m.get(PART_TYPE_KEY))
        .and_then(Value::as_str);
    let body = match tag {
        Some("tool_result") => data,
        Some(_) => return None,
        None => match (
            data.get("part_type").and_then(Value::as_str),
            data.get("data"),
        ) {
            (Some("tool_result"), Some(inner)) => inner,
            (Some(_), Some(_)) => return None,
            _ => data,
        },
    };
    let id = body.get("tool_call_id")?.as_str()?.to_string();
    let name = body
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(parts) = body.get("parts") {
        if let Ok(parts) = serde_json::from_value::<Vec<Part>>(parts.clone()) {
            return Some(ToolResponse::from_parts(id, name, parts));
        }
    }
    let result = body.get("result").cloned().unwrap_or(Value::Null);
    Some(ToolResponse::from_parts(id, name, vec![Part::Data(result)]))
}

/// The tool result a text answer gives its one pending call.
pub fn text_result(call: &WaitingCall, text: &str) -> ToolResponse {
    ToolResponse::from_parts(
        call.tool_call_id.clone(),
        call.tool_name.clone(),
        vec![Part::Data(json!({ "text": text }))],
    )
}

impl AgentOrchestrator {
    /// The client tool calls `task_id` waits on now, in call order: its open
    /// pending calls (durable) and the calls its in-memory run waits on
    /// (non-durable). Empty for a task the caller cannot read.
    pub async fn waiting_calls(&self, task_id: &str) -> Vec<WaitingCall> {
        let Ok(Some(task)) = self.stores.task_store.get_task(task_id).await else {
            return Vec::new();
        };
        let mut calls: Vec<WaitingCall> = Vec::new();
        if let Some(durable) = &self.durable {
            for c in durable
                .pending_calls
                .list_pending(task_id)
                .await
                .unwrap_or_default()
            {
                if !matches!(
                    c.kind,
                    distri_types::PendingCallKind::ClientTool
                        | distri_types::PendingCallKind::Approval
                ) {
                    continue;
                }
                calls.push(WaitingCall {
                    tool_call_id: c.id.clone(),
                    tool_name: c
                        .tool_name
                        .clone()
                        .or_else(|| {
                            c.request
                                .get("tool_name")
                                .and_then(Value::as_str)
                                .map(String::from)
                        })
                        .unwrap_or_default(),
                    input: c.request.get("input").cloned().unwrap_or(Value::Null),
                });
            }
        }
        // Non-durable: the calls registered in the external tool calls
        // store for this task, named from the task's recorded tool calls.
        let store = &self.stores.external_tool_calls_store;
        let mut in_memory = Vec::new();
        for id in store.list_pending_tool_calls().await.unwrap_or_default() {
            if calls.iter().any(|c| c.tool_call_id == id) {
                continue;
            }
            if let Ok(Some(t)) = store.pending_task_id(&id).await {
                if t == task_id {
                    in_memory.push(id);
                }
            }
        }
        if !in_memory.is_empty() {
            let recorded = self.recorded_tool_calls(&task).await;
            // Call order: as the run recorded them.
            for c in &recorded {
                if in_memory.contains(&c.tool_call_id) {
                    calls.push(WaitingCall {
                        tool_call_id: c.tool_call_id.clone(),
                        tool_name: c.tool_name.clone(),
                        input: c.input.clone(),
                    });
                }
            }
            for id in in_memory {
                if !calls.iter().any(|c| c.tool_call_id == id) {
                    calls.push(WaitingCall {
                        tool_call_id: id,
                        tool_name: String::new(),
                        input: Value::Null,
                    });
                }
            }
        }
        calls
    }

    /// The tool calls a task's history records (events and messages).
    async fn recorded_tool_calls(&self, task: &distri_types::Task) -> Vec<ToolCall> {
        let history = self
            .stores
            .task_store
            .get_history(&task.thread_id, None)
            .await
            .unwrap_or_default();
        let mut out: Vec<ToolCall> = Vec::new();
        let mut push = |c: &ToolCall| {
            if !out.iter().any(|o| o.tool_call_id == c.tool_call_id) {
                out.push(c.clone());
            }
        };
        for (_, msgs) in history.iter().filter(|(row, _)| row.id == task.id) {
            for m in msgs {
                match m {
                    distri_types::TaskMessage::Event(e) => {
                        if let distri_types::AgentEventType::ToolCalls { tool_calls, .. } = &e.event
                        {
                            tool_calls.iter().for_each(&mut push);
                        }
                    }
                    distri_types::TaskMessage::Message(m) => {
                        for p in &m.parts {
                            if let Part::ToolCall(c) = p {
                                push(c);
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(v: Value) -> Vec<A2aPart> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn tagged_and_plain_data_parts_are_results() {
        let tagged = parts(json!([{
            "data": { "tool_call_id": "c1", "tool_name": "pick", "result": { "v": 1 } },
            "metadata": { "distri/part_type": "tool_result" }
        }]));
        let AnswerParts::Results(r) = answer_parts(&tagged) else {
            panic!("tagged result")
        };
        assert_eq!(r[0].tool_call_id, "c1");
        assert_eq!(r[0].parts, vec![Part::Data(json!({ "v": 1 }))]);

        let plain = parts(json!([{ "data": { "tool_call_id": "c2", "result": "blue" } }]));
        let AnswerParts::Results(r) = answer_parts(&plain) else {
            panic!("plain result")
        };
        assert_eq!(r[0].tool_call_id, "c2");
        assert_eq!(r[0].tool_name, "");

        let legacy = parts(json!([{ "data": { "part_type": "tool_result", "data": {
            "tool_call_id": "c3", "tool_name": "pick", "parts": [{ "part_type": "text", "data": "x" }]
        } } }]));
        let AnswerParts::Results(r) = answer_parts(&legacy) else {
            panic!("0.x wrapper")
        };
        assert_eq!(r[0].parts, vec![Part::Text("x".into())]);
    }

    #[test]
    fn text_only_is_text_and_other_parts_are_not_answers() {
        assert_eq!(
            answer_parts(&parts(json!([{ "text": "teal" }]))),
            AnswerParts::Text("teal".into())
        );
        assert_eq!(
            answer_parts(&parts(
                json!([{ "text": "a" }, { "data": { "city": "x" } }])
            )),
            AnswerParts::None
        );
        let call = parts(json!([{
            "data": { "tool_call_id": "c1", "tool_name": "pick", "input": {} },
            "metadata": { "distri/part_type": "tool_call" }
        }]));
        assert_eq!(answer_parts(&call), AnswerParts::None);
    }
}
