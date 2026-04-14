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
    build_message_params_full(
        content,
        thread_id,
        task_id,
        model,
        false,
        connections_context,
        None,
    )
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
    // With --remote, the agent will execute on the server (forked into a
    // sandbox by runtime-constraint dispatch). The CLI is just a passthrough
    // for events — it is NOT the execution environment, so it must declare
    // `runtime_mode = Cloud`. Sending `Cli` here causes the server to think
    // the caller already provides Cli runtime, skip the sandbox, run the
    // agent in-process, and 120s-timeout on tool calls (the outer CLI never
    // bound a registry to handle them).
    //
    // Without --remote, the CLI is the execution environment for any external
    // tool calls the server delegates back, so it stays in Cli mode.
    let runtime_mode = if remote {
        RuntimeMode::Cloud
    } else {
        RuntimeMode::Cli
    };
    let mut metadata = ExecutorContextMetadata {
        runtime_mode,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_metadata(params: &MessageSendParams) -> ExecutorContextMetadata {
        let raw = params.metadata.as_ref().expect("metadata must be present");
        serde_json::from_value(raw.clone()).expect("metadata must deserialize")
    }

    /// Regression: with `--remote`, the agent runs on the server (forked into a
    /// sandbox). The CLI is not the execution environment, so it must declare
    /// `runtime_mode = Cloud` so the orchestrator's runtime-constraint dispatch
    /// fires. Sending `runtime_mode = Cli` here causes the dispatch to think the
    /// caller already provides Cli runtime, skip the sandbox, run the agent
    /// in-process on the cloud server, and 120s-timeout on tool calls because
    /// the outer CLI never bound a registry.
    #[test]
    fn remote_request_sets_runtime_mode_cloud() {
        let params = build_message_params_full(
            "test task".into(),
            None,
            None,
            None,
            true, // remote
            None,
            None,
        );
        let metadata = parse_metadata(&params);
        assert_eq!(
            metadata.runtime_mode,
            RuntimeMode::Cloud,
            "with --remote, runtime_mode must be Cloud so the server forks to a sandbox"
        );
        assert_eq!(
            metadata
                .definition_overrides
                .as_ref()
                .and_then(|o| o.remote),
            Some(true),
            "with --remote, definition_overrides.remote must be Some(true)"
        );
    }

    /// Without `--remote`, the CLI is the execution environment for any external
    /// tool calls the server delegates back. Stays in `Cli` mode.
    #[test]
    fn local_request_stays_in_cli_mode() {
        let params = build_message_params_full(
            "test task".into(),
            None,
            None,
            None,
            false, // local
            None,
            None,
        );
        let metadata = parse_metadata(&params);
        assert_eq!(metadata.runtime_mode, RuntimeMode::Cli);
        assert!(
            metadata
                .definition_overrides
                .as_ref()
                .and_then(|o| o.remote)
                .is_none(),
            "without --remote, definition_overrides.remote must be unset"
        );
    }
}
