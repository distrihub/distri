use std::sync::Arc;

use distri_types::configuration::{AgentConfig, DbConnectionConfig, MetadataStoreConfig, StoreConfig};
use distri_types::{Message, ModelSettings};

use crate::{
    agent::{parse_agent_markdown_content, ExecutorContext},
    AgentOrchestrator, AgentOrchestratorBuilder,
};

/// Creates a StoreConfig that uses a temporary in-memory SQLite database
/// so tests don't depend on the filesystem having a `.distri/` directory.
fn test_store_config() -> StoreConfig {
    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{}?mode=memory&cache=shared", db_name);
    StoreConfig {
        metadata: MetadataStoreConfig {
            db_config: Some(DbConnectionConfig {
                database_url: db_url,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn test_orchestrator_final_result_capture() {
    if std::env::var("OPENAI_API_KEY").is_err() {
        eprintln!("skipping orchestrator test; OPENAI_API_KEY not set");
        return;
    }
    let agent = parse_agent_markdown_content(include_str!("./test_agent.md"))
        .await
        .unwrap();
    let name = agent.name.clone();
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    let context = Arc::new(ExecutorContext {
        orchestrator: Some(orchestrator.clone()),
        verbose: true,
        ..Default::default()
    });
    orchestrator.register_agent_definition(agent).await.unwrap();
    let result = orchestrator
        .execute(
            &name.as_str(),
            Message::user("Test final result".to_string(), None),
            context,
            None,
        )
        .await;
    assert!(result.is_ok());
    let content = result.unwrap().content;
    println!("Content: {:?}", content);
    assert!(content.is_some());
}

#[tokio::test]
async fn test_agent_inherits_default_model_settings_from_context() {
    // Agent definition without model_settings — should inherit from orchestrator defaults
    let agent_md = r#"---
name = "no_model_agent"
description = "Agent without model settings"
instructions = "You are a test agent."
max_iterations = 1
---
"#;
    let def = parse_agent_markdown_content(agent_md).await.unwrap();
    // Verify the parsed agent has no model_settings
    assert!(
        def.model_settings.is_none(),
        "parsed agent should have no model_settings"
    );

    // Build orchestrator with explicit default model settings
    let default_settings = ModelSettings {
        model: Some("gpt-4o-test".to_string()),
        temperature: Some(0.5),
        ..Default::default()
    };
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_default_model_settings(default_settings.clone())
            .build()
            .await
            .unwrap(),
    );

    // Register the agent (no model_settings)
    orchestrator
        .register_agent_definition(def.clone())
        .await
        .unwrap();

    // Simulate what happens during execution: apply_agent_overrides merges defaults
    let mut agent_config = AgentConfig::StandardAgent(def.clone());
    orchestrator
        .apply_agent_overrides(&mut agent_config, None)
        .await;
    let AgentConfig::StandardAgent(loaded_def) = &agent_config;

    // After apply_agent_overrides, the agent should have the orchestrator's model
    let ms = loaded_def.model_settings();
    assert_eq!(
        ms.model,
        Some("gpt-4o-test".to_string()),
        "agent should inherit model from orchestrator default_model_settings"
    );
    assert_eq!(
        ms.temperature,
        Some(0.5),
        "agent should inherit temperature from orchestrator default_model_settings"
    );

    // Also verify orchestrator stores the default_model_settings for context injection
    let orch_defaults = orchestrator.get_default_model_settings().await;
    assert_eq!(
        orch_defaults.model,
        Some("gpt-4o-test".to_string()),
        "Orchestrator should store default_model_settings for injection into ExecutorContext"
    );
}

#[tokio::test]
async fn test_agent_model_settings_override_defaults() {
    // Agent with its own model — should override the orchestrator default
    let agent_md = r#"---
name = "custom_model_agent"
description = "Agent with custom model"
instructions = "You are a test agent."
max_iterations = 1

[model_settings]
model = "custom-model-v1"
temperature = 0.9
---
"#;
    let def = parse_agent_markdown_content(agent_md).await.unwrap();
    assert_eq!(
        def.model_settings().model,
        Some("custom-model-v1".to_string())
    );

    let default_settings = ModelSettings {
        model: Some("gpt-4o-default".to_string()),
        temperature: Some(0.5),
        max_tokens: Some(1000),
        ..Default::default()
    };
    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .with_default_model_settings(default_settings)
            .build()
            .await
            .unwrap(),
    );
    orchestrator
        .register_agent_definition(def.clone())
        .await
        .unwrap();

    // apply_agent_overrides merges orchestrator defaults with agent settings
    let mut agent_config = AgentConfig::StandardAgent(def.clone());
    orchestrator
        .apply_agent_overrides(&mut agent_config, None)
        .await;
    let AgentConfig::StandardAgent(loaded_def) = &agent_config;
    let ms = loaded_def.model_settings();

    // Agent's model should win
    assert_eq!(ms.model, Some("custom-model-v1".to_string()));
    // Agent's temperature should win
    assert_eq!(ms.temperature, Some(0.9));
    // Default's max_tokens should be inherited (agent didn't set it)
    assert_eq!(ms.max_tokens, Some(1000));
}

#[tokio::test]
async fn test_merge_model_settings_errors_when_no_model() {
    // Both base and agent have no model — merge should error
    let base = ModelSettings::default();
    let agent = ModelSettings::default();
    let sentinel = ModelSettings::default();

    let result = AgentOrchestrator::merge_model_settings(&base, &agent, &sentinel);
    assert!(
        result.is_err(),
        "merge_model_settings should error when no model is set"
    );
}
