//! `distri.yaml` — the OSS standalone server's declarative seed config.
//!
//! It is the OSS equivalent of cloud workspace settings + seed data, layered
//! on top of the built-in basics in `default_models.json`:
//!
//! - `model_providers` — provider/model extensions (layer 2 of the provider
//!   registry). Folded in via [`distri_types::register_provider_extensions`].
//! - `default_model` — seeded into the runtime store when none is set yet.
//! - `agents` — agent definition files to load and register on startup.
//!
//! The file is optional; an absent `distri.yaml` leaves the server on the
//! built-in basics (`openai`, `anthropic`, `gemini`).

use anyhow::{Context, Result};
use distri_core::AgentOrchestrator;
use distri_types::configuration::AgentConfig;
use distri_types::stores::UpsertProviderRequest;
use distri_types::ModelProviderDefinition;
use serde::Deserialize;
use std::path::Path;

/// File name looked up in the workspace directory.
const DISTRI_YAML: &str = "distri.yaml";

/// Parsed `distri.yaml`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DistriYamlConfig {
    /// Provider/model definitions layered onto the built-in basics.
    pub model_providers: Vec<ModelProviderDefinition>,
    /// Default model in `provider/model` form. Seeded only when the runtime
    /// store has no default model yet.
    pub default_model: Option<String>,
    /// Agent definition files to load and register on startup.
    pub agents: Vec<AgentSeed>,
}

/// A single agent seed entry.
#[derive(Debug, Deserialize)]
pub struct AgentSeed {
    /// Path to an agent markdown file, relative to the workspace directory.
    pub file: String,
}

/// Load `distri.yaml` from the workspace directory, if present.
pub fn load(workspace_path: &Path) -> Result<Option<DistriYamlConfig>> {
    let path = workspace_path.join(DISTRI_YAML);
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let config: DistriYamlConfig =
        serde_yaml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    tracing::info!(
        "loaded {} ({} provider extension(s), {} agent seed(s))",
        path.display(),
        config.model_providers.len(),
        config.agents.len(),
    );
    Ok(Some(config))
}

/// Fold `distri.yaml`'s provider extensions into the global provider
/// registry. Call once, before the server serves the model/provider catalog.
pub fn register_extensions(config: &DistriYamlConfig) {
    if config.model_providers.is_empty() {
        return;
    }
    distri_types::register_provider_extensions(config.model_providers.clone());
}

/// Apply the runtime-mutable seeds (default model, agents) after the
/// orchestrator is built. `distri.yaml` is declarative seed data — the
/// default model is only set when the runtime store has none.
pub async fn apply_runtime_seeds(
    config: &DistriYamlConfig,
    orchestrator: &AgentOrchestrator,
    workspace_path: &Path,
) -> Result<()> {
    seed_default_model(config, orchestrator).await;
    seed_agents(config, orchestrator, workspace_path).await;
    Ok(())
}

async fn seed_default_model(config: &DistriYamlConfig, orchestrator: &AgentOrchestrator) {
    let Some(model) = config
        .default_model
        .as_deref()
        .filter(|m| !m.trim().is_empty())
    else {
        return;
    };
    let Some(provider_store) = orchestrator.stores.provider_store.as_ref() else {
        tracing::warn!("distri.yaml default_model set but no provider store; skipping");
        return;
    };
    match provider_store.get_default_model().await {
        Ok(Some(_)) => {
            tracing::debug!("default model already set; not overriding distri.yaml seed");
        }
        Ok(None) => {
            let provider_id = model.split('/').next().unwrap_or(model).to_string();
            let req = UpsertProviderRequest {
                provider_id,
                secrets: Default::default(),
                config: None,
                custom_models: None,
                default_model: Some(model.to_string()),
                connection_provider: None,
            };
            match provider_store.upsert_provider(req).await {
                Ok(_) => tracing::info!("seeded default model from distri.yaml: {model}"),
                Err(e) => tracing::warn!("failed to seed default model from distri.yaml: {e}"),
            }
        }
        Err(e) => tracing::warn!("could not read default model: {e}"),
    }
}

async fn seed_agents(
    config: &DistriYamlConfig,
    orchestrator: &AgentOrchestrator,
    workspace_path: &Path,
) {
    for seed in &config.agents {
        let path = workspace_path.join(&seed.file);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("distri.yaml agent seed {}: {e}", path.display());
                continue;
            }
        };
        match distri_types::parse_agent_markdown_content(&content).await {
            Ok(def) => {
                let name = def.name.clone();
                match orchestrator
                    .stores
                    .agent_store
                    .register(AgentConfig::StandardAgent(def))
                    .await
                {
                    Ok(()) => tracing::info!("seeded agent from distri.yaml: {name}"),
                    Err(e) => {
                        tracing::warn!("failed to register agent '{name}' from distri.yaml: {e}")
                    }
                }
            }
            Err(e) => tracing::warn!("failed to parse agent {}: {e:?}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full `distri.yaml` deserializes into all three sections, and a
    /// model entry that omits `name` is accepted (backfilled downstream).
    #[test]
    fn parses_full_distri_yaml() {
        let yaml = r#"
model_providers:
  - id: azure_ai_foundry
    label: Azure AI Foundry
    keys:
      - key: AZURE_AI_FOUNDRY_RESOURCE
        label: Resource name
        sensitive: false
        url_template: "https://{}.openai.azure.com/openai/v1"
      - key: AZURE_AI_FOUNDRY_API_KEY
        label: API key
        sensitive: true
    models:
      - id: gpt-5.4
        capability: completion
        context_window: 128000
default_model: openai/gpt-4.1-mini
agents:
  - file: agents/coder.md
"#;
        let config: DistriYamlConfig = serde_yaml::from_str(yaml).expect("distri.yaml parses");
        assert_eq!(config.model_providers.len(), 1);
        assert_eq!(config.model_providers[0].id, "azure_ai_foundry");
        assert_eq!(config.model_providers[0].keys.len(), 2);
        assert_eq!(config.default_model.as_deref(), Some("openai/gpt-4.1-mini"));
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].file, "agents/coder.md");
    }

    /// Every section is optional — an empty file is a valid (no-op) config.
    #[test]
    fn parses_empty_distri_yaml() {
        let config: DistriYamlConfig = serde_yaml::from_str("{}").expect("empty config parses");
        assert!(config.model_providers.is_empty());
        assert!(config.default_model.is_none());
        assert!(config.agents.is_empty());
    }
}
