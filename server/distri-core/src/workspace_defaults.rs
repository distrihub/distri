use crate::agent::AgentOrchestrator;
use distri_types::ModelSettings;
use serde::Deserialize;
use std::sync::Arc;

const DEFAULT_MODEL_SECRET_KEY: &str = "DISTRI_DEFAULT_MODEL";

#[derive(Debug, Deserialize)]
struct WorkspaceConfigFile {
    #[serde(default)]
    model_settings: Option<ModelSettings>,
}

fn load_model_from_distri_toml() -> Option<ModelSettings> {
    let path = std::env::current_dir().ok()?.join("distri.toml");
    let raw = std::fs::read_to_string(path).ok()?;
    let cfg: WorkspaceConfigFile = toml::from_str(&raw).ok()?;
    cfg.model_settings
}

async fn load_model_from_secret_store(executor: &Arc<AgentOrchestrator>) -> Option<ModelSettings> {
    let store = executor.stores.secret_store.as_ref()?;
    let rec = store.get(DEFAULT_MODEL_SECRET_KEY).await.ok().flatten()?;
    let raw = rec.value.trim();
    if raw.is_empty() {
        return None;
    }

    match ModelSettings::from_provider_model_str(raw) {
        Ok(Some(ms)) => Some(ms),
        Ok(None) => {
            tracing::warn!(
                "Invalid {} format '{}'; expected 'provider/model'",
                DEFAULT_MODEL_SECRET_KEY,
                raw
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                "Failed to parse {}='{}': {}",
                DEFAULT_MODEL_SECRET_KEY,
                raw,
                e
            );
            None
        }
    }
}

/// Resolve workspace default model settings for OSS/server execution.
///
/// Resolution order:
/// 1) `DISTRI_DEFAULT_MODEL` from SecretStore (written by `/v1/providers`)
/// 2) `[model_settings]` from `./distri.toml`
pub async fn load_workspace_default_model_settings(
    executor: &Arc<AgentOrchestrator>,
) -> Option<ModelSettings> {
    if let Some(ms) = load_model_from_secret_store(executor).await {
        return Some(ms);
    }
    load_model_from_distri_toml()
}
