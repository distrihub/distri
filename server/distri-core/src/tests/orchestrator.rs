use std::sync::Arc;

use distri_types::configuration::{
    AgentConfig, DbConnectionConfig, MetadataStoreConfig, StoreConfig,
};
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

fn test_model_settings(model: &str) -> ModelSettings {
    ModelSettings::new(model)
}

#[tokio::test]
async fn test_default_model_settings_injected_into_agent() {
    // Agent with NO model_settings — should get defaults from context
    let agent_md = r#"---
name = "no_model_agent"
description = "Agent without explicit model"
instructions = "You are a test agent."
max_iterations = 1
---
"#;
    let def = parse_agent_markdown_content(agent_md).await.unwrap();
    assert!(
        def.model_settings.is_none(),
        "parsed agent should have no model_settings"
    );

    let default_settings = ModelSettings {
        model: "gpt-4o-test".to_string(),
        inner: distri_types::ModelSettingsInner {
            temperature: Some(0.5),
            ..Default::default()
        },
    };

    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );

    orchestrator
        .register_agent_definition(def.clone())
        .await
        .unwrap();

    // apply_agent_overrides merges defaults with agent settings
    let mut agent_config = AgentConfig::StandardAgent(def.clone());
    let defaults = Some(default_settings.clone());
    AgentOrchestrator::apply_agent_overrides(&mut agent_config, None, &defaults);
    let AgentConfig::StandardAgent(loaded_def) = &agent_config;

    let ms = loaded_def
        .model_settings()
        .expect("model_settings should be set after merge");
    assert_eq!(ms.model, "gpt-4o-test");
    assert_eq!(ms.inner.temperature, Some(0.5));
}

#[tokio::test]
async fn test_agent_model_settings_override_defaults() {
    // Agent with its own model — should override the default
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
    assert_eq!(def.model_settings().unwrap().model, "custom-model-v1");

    let default_settings = ModelSettings {
        model: "gpt-4o-default".to_string(),
        inner: distri_types::ModelSettingsInner {
            max_tokens: Some(1000),
            ..Default::default()
        },
    };

    let orchestrator = Arc::new(
        AgentOrchestratorBuilder::default()
            .with_store_config(test_store_config())
            .build()
            .await
            .unwrap(),
    );
    orchestrator
        .register_agent_definition(def.clone())
        .await
        .unwrap();

    let mut agent_config = AgentConfig::StandardAgent(def.clone());
    let defaults = Some(default_settings);
    AgentOrchestrator::apply_agent_overrides(&mut agent_config, None, &defaults);
    let AgentConfig::StandardAgent(loaded_def) = &agent_config;
    let ms = loaded_def
        .model_settings()
        .expect("model_settings should be set after merge");

    // Agent's model should win
    assert_eq!(ms.model, "custom-model-v1");
    // Agent's temperature should win
    assert_eq!(ms.inner.temperature, Some(0.9));
    // Default's max_tokens should be inherited (agent didn't set it)
    assert_eq!(ms.inner.max_tokens, Some(1000));
}

#[tokio::test]
async fn test_merge_model_settings_errors_when_no_model() {
    // Both base and agent have no model — merge should error
    let base = test_model_settings("");
    let agent = test_model_settings("");

    let result = AgentOrchestrator::merge_model_settings(&base, &agent);
    assert!(
        result.is_err(),
        "merge_model_settings should error when no model is set"
    );
}
