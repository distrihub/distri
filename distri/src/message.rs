//! Helpers for building the `MessageSendParams` that both the CLI and the
//! server-side in-process runner send when kicking off an agent run.
//!
//! Both call paths go through these builders so the runtime_mode /
//! definition_overrides semantics can't drift between them.

use crate::Distri;
use distri_a2a::{
    EventKind, Message as A2aMessage, MessageSendParams, Part as A2aPart, Role, TextPart,
};
use distri_types::configuration::DefinitionOverrides;
use distri_types::{ExecutorContextMetadata, RuntimeMode};
use std::collections::HashMap;

/// Build a lightweight connections summary to inject into the agent's prompt context.
pub async fn build_connections_context(client: &Distri) -> Option<String> {
    let connections = client.list_connections().await.ok()?;
    if connections.is_empty() {
        return None;
    }
    let lines: Vec<String> = connections
        .iter()
        .filter(|c| c.status.as_deref() == Some("connected"))
        .map(|c| {
            let scopes: Vec<String> = c
                .config
                .as_ref()
                .and_then(|cfg| cfg.get("scopes"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            format!(
                "- **{}** (connection_id: `{}`): scopes=[{}]",
                c.name,
                c.id,
                scopes.join(", ")
            )
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

pub fn build_message_params(
    content: String,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    model: Option<&str>,
    connections_context: Option<String>,
) -> MessageSendParams {
    build_message_params_full(
        content,
        thread_id,
        task_id,
        model,
        connections_context,
        None,
        None,
        None,
    )
}

pub fn build_message_params_full(
    content: String,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    model: Option<&str>,
    connections_context: Option<String>,
    env_vars: Option<HashMap<String, String>>,
    tags: Option<HashMap<String, String>>,
    trace_context: Option<distri_types::TraceContext>,
) -> MessageSendParams {
    let has_overrides = model.is_some();
    // The CLI is always the execution environment for any external tool
    // calls the server delegates back — there is no remote/sandbox mode
    // where a separate container executes them instead. Always declare
    // `runtime_mode = Cli`.
    let mut metadata = ExecutorContextMetadata {
        runtime_mode: RuntimeMode::Cli,
        definition_overrides: if has_overrides {
            Some(DefinitionOverrides {
                model: model.map(|m| m.to_string()),
                ..Default::default()
            })
        } else {
            None
        },
        env_vars,
        tags,
        trace_context,
        ..Default::default()
    };

    if let Some(conn_ctx) = connections_context {
        let mut dv = HashMap::new();
        dv.insert(
            "available_connections".to_string(),
            serde_json::Value::String(conn_ctx),
        );
        metadata.dynamic_values = Some(dv);
    }

    MessageSendParams {
        message: A2aMessage {
            kind: EventKind::Message,
            message_id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![A2aPart::Text(TextPart { text: content })],
            context_id: Some(
                thread_id
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ),
            task_id: Some(
                task_id
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ),
            reference_task_ids: vec![],
            extensions: vec![],
            metadata: None,
        },
        configuration: None,
        metadata: Some(serde_json::to_value(&metadata).unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_metadata(params: &MessageSendParams) -> ExecutorContextMetadata {
        let raw = params.metadata.as_ref().expect("metadata must be present");
        serde_json::from_value(raw.clone()).expect("metadata must deserialize")
    }

    /// The CLI is always the execution environment for any external tool
    /// calls the server delegates back — there is no remote/sandbox mode.
    /// Every request declares `runtime_mode = Cli` and carries no `runtime`
    /// override.
    #[test]
    fn request_stays_in_cli_mode() {
        let params = build_message_params_full(
            "test task".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let metadata = parse_metadata(&params);
        assert_eq!(metadata.runtime_mode, RuntimeMode::Cli);
        assert!(
            metadata
                .definition_overrides
                .as_ref()
                .and_then(|o| o.runtime.as_ref())
                .is_none(),
            "definition_overrides.runtime must be unset"
        );
    }
}
