use distri::Distri;
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
    build_message_params_full(content, thread_id, task_id, model, false, connections_context, None)
}

pub fn build_message_params_full(
    content: String,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    model: Option<&str>,
    remote: bool,
    connections_context: Option<String>,
    env_vars: Option<HashMap<String, String>>,
) -> MessageSendParams {
    let has_overrides = model.is_some() || remote;
    let mut metadata = ExecutorContextMetadata {
        runtime_mode: RuntimeMode::Cli,
        definition_overrides: if has_overrides {
            Some(DefinitionOverrides {
                model: model.map(|m| m.to_string()),
                remote: if remote { Some(true) } else { None },
                ..Default::default()
            })
        } else {
            None
        },
        env_vars,
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
