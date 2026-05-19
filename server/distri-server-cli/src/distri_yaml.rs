//! `distri.yaml` — the OSS standalone server's declarative seed config.
//!
//! It is the OSS equivalent of cloud workspace settings + seed data, layered
//! on top of the built-in basics in `default_models.json`:
//!
//! - `model_providers` — provider/model extensions inline, in the catalog
//!   section format (`completion` / `tts` / `stt`).
//! - `model_providers_path` — a directory of per-provider catalog files, or
//!   a single combined file (e.g. a GitHub-release artifact).
//! - `default_model` — seeded into the runtime store when none is set yet.
//! - `agents` — agent definition files to load and register on startup.
//!
//! Provider extensions are also picked up, with no `distri.yaml` needed,
//! from a `providers/` directory in the workspace and from the
//! `DISTRI_MODEL_CATALOG` env var (a directory or combined file). All
//! sources fold into layer 2 of the provider registry.

use anyhow::{Context, Result};
use distri_core::AgentOrchestrator;
use distri_types::configuration::AgentConfig;
use distri_types::model_catalog::{self, ProviderCatalogEntry};
use distri_types::stores::UpsertProviderRequest;
use serde::Deserialize;
use std::path::Path;

/// File name looked up in the workspace directory.
const DISTRI_YAML: &str = "distri.yaml";
/// Directory of per-provider catalog files, auto-loaded from the workspace.
const PROVIDERS_DIR: &str = "providers";
/// Env var pointing to an extra catalog (directory or combined file).
const CATALOG_ENV: &str = "DISTRI_MODEL_CATALOG";

/// Parsed `distri.yaml`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DistriYamlConfig {
    /// Provider/model definitions inline, in the catalog section format.
    pub model_providers: Vec<ProviderCatalogEntry>,
    /// A directory of per-provider catalog files, or a single combined file.
    /// Relative paths resolve against the workspace directory.
    pub model_providers_path: Option<String>,
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
    tracing::info!("loaded {}", path.display());
    Ok(Some(config))
}

/// Gather provider/model extensions from every source and fold them into the
/// global provider registry. Call once, before the server serves the
/// catalog. Sources, lowest-to-highest precedence on `id` collisions:
/// the workspace `providers/` directory, `distri.yaml`'s inline
/// `model_providers`, its `model_providers_path`, and `DISTRI_MODEL_CATALOG`.
pub fn register_extensions(workspace_path: &Path, config: Option<&DistriYamlConfig>) {
    let mut entries: Vec<ProviderCatalogEntry> = Vec::new();

    let providers_dir = workspace_path.join(PROVIDERS_DIR);
    if providers_dir.is_dir() {
        match model_catalog::load_provider_dir(&providers_dir) {
            Ok(loaded) => entries.extend(loaded),
            Err(e) => tracing::warn!("failed to load {}: {e}", providers_dir.display()),
        }
    }

    if let Some(config) = config {
        entries.extend(config.model_providers.iter().cloned());

        if let Some(path) = config
            .model_providers_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            let resolved = workspace_path.join(path);
            match model_catalog::load_catalog_path(&resolved) {
                Ok(loaded) => entries.extend(loaded),
                Err(e) => tracing::warn!("failed to load {}: {e}", resolved.display()),
            }
        }
    }

    if let Ok(path) = std::env::var(CATALOG_ENV) {
        let path = path.trim();
        if !path.is_empty() {
            match model_catalog::load_catalog_path(Path::new(path)) {
                Ok(loaded) => entries.extend(loaded),
                Err(e) => tracing::warn!("failed to load {CATALOG_ENV}={path}: {e}"),
            }
        }
    }

    if entries.is_empty() {
        return;
    }
    tracing::info!("registering {} provider extension(s)", entries.len());
    model_catalog::register(entries);
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

    /// A full `distri.yaml` deserializes into all sections, including the
    /// catalog section format for inline providers.
    #[test]
    fn parses_full_distri_yaml() {
        let yaml = r#"
model_providers:
  - id: azure_ai_foundry
    label: Azure AI Foundry
    keys:
      - { key: AZURE_AI_FOUNDRY_RESOURCE, label: Resource name, sensitive: false,
          url_template: "https://{}.openai.azure.com/openai/v1" }
      - { key: AZURE_AI_FOUNDRY_API_KEY, label: API key, sensitive: true }
    completion:
      - { id: gpt-5.4, name: GPT-5.4, context_window: 128000,
          pricing: { type: completion, input: 5.0, output: 15.0 } }
    tts:
      - { id: gpt-4o-mini-tts, pricing: { type: tts, per_1m_chars: 12.0 } }
model_providers_path: providers
default_model: openai/gpt-4.1-mini
agents:
  - file: agents/coder.md
"#;
        let config: DistriYamlConfig = serde_yaml::from_str(yaml).expect("distri.yaml parses");
        assert_eq!(config.model_providers.len(), 1);
        assert_eq!(config.model_providers[0].id, "azure_ai_foundry");
        assert_eq!(config.model_providers[0].completion.len(), 1);
        assert_eq!(config.model_providers[0].tts.len(), 1);
        assert_eq!(config.model_providers_path.as_deref(), Some("providers"));
        assert_eq!(config.default_model.as_deref(), Some("openai/gpt-4.1-mini"));
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].file, "agents/coder.md");
    }

    /// Every section is optional — an empty file is a valid (no-op) config.
    #[test]
    fn parses_empty_distri_yaml() {
        let config: DistriYamlConfig = serde_yaml::from_str("{}").expect("empty config parses");
        assert!(config.model_providers.is_empty());
        assert!(config.model_providers_path.is_none());
        assert!(config.default_model.is_none());
        assert!(config.agents.is_empty());
    }
}
