use std::sync::Arc;

use distri_types::configuration::AgentConfig;
use distri_types::ModelSettings;

use crate::tests::helpers::test_store_config;
use crate::{agent::parse_agent_markdown_content, AgentOrchestrator, AgentOrchestratorBuilder};

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
    let AgentConfig::StandardAgent(loaded_def) = &agent_config else {
        panic!("expected StandardAgent")
    };

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
    let AgentConfig::StandardAgent(loaded_def) = &agent_config else {
        panic!("expected StandardAgent")
    };
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
    // Both base and agent have no model — merge should return None
    let base = test_model_settings("");
    let agent = test_model_settings("");

    let result = base.merge(&agent);
    assert!(
        result.is_none(),
        "merge should return None when no model is set"
    );
}

#[tokio::test]
async fn test_merge_custom_provider_workspace_overrides_agent_model() {
    // When workspace uses a custom provider (e.g. Azure via OpenAICompatible),
    // the agent's bare model name should NOT override the workspace model,
    // because the agent's model may not be available on the custom provider.
    let base = ModelSettings {
        model: "gpt-5.4".to_string(),
        inner: distri_types::ModelSettingsInner {
            provider: distri_types::ModelProvider::OpenAICompatible {
                base_url: "https://custom.azure.com/openai/v1".to_string(),
                api_key: Some("test-key".to_string()),
                project_id: None,
            },
            ..Default::default()
        },
    };
    let agent = ModelSettings {
        model: "gpt-5.1".to_string(),
        inner: distri_types::ModelSettingsInner {
            // No explicit provider — serde defaults to OpenAI
            ..Default::default()
        },
    };

    let result = base.merge(&agent).unwrap();

    // Workspace model should win because workspace uses a custom provider
    // and agent did not explicitly set a provider
    assert_eq!(
        result.model, "gpt-5.4",
        "workspace model should take precedence when workspace uses custom provider"
    );
    // Workspace provider should be used
    assert!(
        matches!(
            result.inner.provider,
            distri_types::ModelProvider::OpenAICompatible { .. }
        ),
        "workspace custom provider should be used"
    );
}

#[tokio::test]
async fn test_merge_agent_explicit_provider_overrides_workspace() {
    // When agent explicitly sets a provider (e.g. Anthropic),
    // agent's model and provider should win.
    let base = ModelSettings {
        model: "gpt-5.4".to_string(),
        inner: distri_types::ModelSettingsInner {
            provider: distri_types::ModelProvider::OpenAICompatible {
                base_url: "https://custom.azure.com/openai/v1".to_string(),
                api_key: Some("test-key".to_string()),
                project_id: None,
            },
            ..Default::default()
        },
    };
    let agent = ModelSettings {
        model: "claude-sonnet-4".to_string(),
        inner: distri_types::ModelSettingsInner {
            provider: distri_types::ModelProvider::Anthropic {
                base_url: None,
                api_key: None,
            },
            ..Default::default()
        },
    };

    let result = base.merge(&agent).unwrap();

    // Agent's model and provider should win since it explicitly set a provider
    assert_eq!(result.model, "claude-sonnet-4");
    assert!(
        matches!(
            result.inner.provider,
            distri_types::ModelProvider::Anthropic { .. }
        ),
        "agent's explicit provider should be used"
    );
}
