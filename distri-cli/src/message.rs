use distri::Distri;
use distri_a2a::{
    EventKind, Message as A2aMessage, MessageSendParams, Part as A2aPart, Role, TextPart,
};

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
    connections_context: Option<String>,
) -> MessageSendParams {
    let mut meta = serde_json::json!({});
    if let Some(conn_ctx) = connections_context {
        meta["dynamic_values"] = serde_json::json!({
            "available_connections": conn_ctx
        });
    }
    MessageSendParams {
        message: A2aMessage {
            kind: EventKind::Message,
            message_id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![A2aPart::Text(TextPart { text: content })],
            context_id: None,
            task_id: None,
            reference_task_ids: vec![],
            extensions: vec![],
            metadata: None,
        },
        configuration: None,
        metadata: Some(meta),
    }
}

pub fn build_chat_message_params(
    content: String,
    thread_id: &str,
    model: Option<&str>,
    connections_context: Option<String>,
) -> MessageSendParams {
    let mut meta = serde_json::json!({});
    if let Some(model) = model {
        meta["definition_overrides"] = serde_json::json!({ "model": model });
    }
    if let Some(conn_ctx) = connections_context {
        let dv = meta
            .get("dynamic_values")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut dv = dv;
        dv.insert(
            "available_connections".to_string(),
            serde_json::Value::String(conn_ctx),
        );
        meta["dynamic_values"] = serde_json::Value::Object(dv);
    }

    MessageSendParams {
        message: A2aMessage {
            kind: EventKind::Message,
            message_id: uuid::Uuid::new_v4().to_string(),
            role: Role::User,
            parts: vec![A2aPart::Text(TextPart { text: content })],
            context_id: Some(thread_id.to_string()),
            task_id: None,
            reference_task_ids: vec![],
            extensions: vec![],
            metadata: None,
        },
        configuration: None,
        metadata: Some(meta),
    }
}
