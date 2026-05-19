//! Secret-backed `ProviderStore` implementation for the standalone OSS server.
//!
//! The standalone `distri-server` is single-tenant and has no `workspaces`
//! table, so there is nowhere to persist a `WorkspaceSettings`-style record.
//! Provider API keys are stored as plain secrets, and the workspace "default
//! model" is persisted as a reserved secret key (`DISTRI_DEFAULT_MODEL`).
//!
//! The multi-tenant cloud uses a different `ProviderStore` impl
//! (`PgProviderStore`) that writes to `workspaces.settings`. Both are reached
//! through the same `/providers` routes via `web::Data<Arc<dyn ProviderStore>>`
//! — the routes never know which backend they are talking to.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use distri_types::stores::{
    NewSecret, ProviderStore, SecretStore, UpsertProviderRequest, UpsertProviderResponse,
};
use distri_types::ModelProvider;

/// Reserved secret key under which the standalone server persists the
/// workspace default model (`"provider/model"` format).
const DEFAULT_MODEL_SECRET_KEY: &str = "DISTRI_DEFAULT_MODEL";

/// `ProviderStore` backed by a plain `SecretStore`. Used by the standalone
/// OSS server, which has no per-workspace settings record.
pub struct SecretBackedProviderStore {
    secret_store: Arc<dyn SecretStore>,
}

impl SecretBackedProviderStore {
    pub fn new(secret_store: Arc<dyn SecretStore>) -> Self {
        Self { secret_store }
    }

    /// Upsert a single secret by key (update when present, create otherwise).
    async fn upsert_secret(&self, key: &str, value: &str) -> Result<()> {
        match self.secret_store.get(key).await? {
            Some(_) => self.secret_store.update(key, value).await.map(|_| ()),
            None => self
                .secret_store
                .create(NewSecret {
                    key: key.to_string(),
                    value: value.to_string(),
                })
                .await
                .map(|_| ()),
        }
    }
}

#[async_trait]
impl ProviderStore for SecretBackedProviderStore {
    async fn upsert_provider(&self, req: UpsertProviderRequest) -> Result<UpsertProviderResponse> {
        // 1. Persist provider secrets (upsert by key).
        for (key, value) in &req.secrets {
            self.upsert_secret(key, value).await?;
        }
        let secrets_saved = req.secrets.len();

        // 2. Persist the default model, or clear it on an empty string.
        //    `config` / `custom_models` / `connection_provider` are not
        //    supported by the single-tenant server (no settings record).
        if let Some(default_model) = req.default_model.as_ref() {
            let trimmed = default_model.trim();
            if trimmed.is_empty() {
                // Clearing is idempotent: missing key is not an error.
                if self.secret_store.get(DEFAULT_MODEL_SECRET_KEY).await?.is_some() {
                    self.secret_store.delete(DEFAULT_MODEL_SECRET_KEY).await?;
                }
            } else {
                self.upsert_secret(DEFAULT_MODEL_SECRET_KEY, trimmed).await?;
            }
        }

        Ok(UpsertProviderResponse {
            provider_id: req.provider_id,
            secrets_saved,
            config_saved: req.default_model.is_some(),
        })
    }

    async fn delete_provider(&self, provider_id: &str) -> Result<()> {
        // Delete every secret key declared by the built-in provider
        // definition. Custom providers have no definition here, so there is
        // nothing to delete — the single-tenant server cannot register them.
        if let Some(def) = ModelProvider::all_provider_definitions()
            .into_iter()
            .find(|d| d.id == provider_id)
        {
            for key in def.keys {
                if self.secret_store.get(&key.key).await?.is_some() {
                    self.secret_store.delete(&key.key).await?;
                }
            }
        }
        Ok(())
    }

    async fn get_default_model(&self) -> Result<Option<String>> {
        Ok(self
            .secret_store
            .get(DEFAULT_MODEL_SECRET_KEY)
            .await?
            .map(|s| s.value))
    }
}
