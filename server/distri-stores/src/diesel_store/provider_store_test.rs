//! Service tests for the sqlite-backed `DieselServerSettingsProviderStore`.
//!
//! Mirrors the cloud's `PgProviderStore` tests (`test_provider_config.rs`):
//! provider secrets land in the `secrets` table, provider settings (default
//! model, custom providers/models) land in the single `server_settings` row,
//! and `delete_provider` cleans up all three plus the default-model pointer.

#![cfg(test)]
#![cfg(feature = "sqlite")]

use std::collections::HashMap;

use crate::diesel_store::{DieselStoreBuilder, SqliteConnectionWrapper};
use distri_types::stores::{
    CustomModelEntry, CustomProviderConfig, ProviderStore, SecretStore, UpsertProviderRequest,
};

async fn test_store() -> DieselStoreBuilder<SqliteConnectionWrapper> {
    let db_name = uuid::Uuid::new_v4();
    let db_url = format!("file:{db_name}?mode=memory&cache=shared");
    DieselStoreBuilder::sqlite(&db_url, 1)
        .await
        .expect("failed to create test store")
}

/// Build an `UpsertProviderRequest` with all optional fields empty.
fn upsert_req(provider_id: &str) -> UpsertProviderRequest {
    UpsertProviderRequest {
        provider_id: provider_id.to_string(),
        secrets: HashMap::new(),
        config: None,
        custom_models: None,
        default_model: None,
        connection_provider: None,
    }
}

#[tokio::test]
async fn upsert_provider_persists_secrets_to_secrets_table() {
    let store = test_store().await;
    let provider_store = store.provider_store();
    let secret_store = store.secret_store();

    let mut req = upsert_req("openai");
    req.secrets
        .insert("OPENAI_API_KEY".to_string(), "sk-test-openai".to_string());

    let resp = provider_store
        .upsert_provider(req)
        .await
        .expect("upsert_provider failed");
    assert_eq!(resp.provider_id, "openai");
    assert_eq!(resp.secrets_saved, 1);
    assert!(!resp.config_saved);

    let secret = secret_store
        .get("OPENAI_API_KEY")
        .await
        .unwrap()
        .expect("secret should exist");
    assert_eq!(secret.value, "sk-test-openai");
}

#[tokio::test]
async fn upsert_provider_overwrites_existing_secret() {
    let store = test_store().await;
    let provider_store = store.provider_store();
    let secret_store = store.secret_store();

    let mut first = upsert_req("openai");
    first
        .secrets
        .insert("OPENAI_API_KEY".to_string(), "sk-original".to_string());
    provider_store.upsert_provider(first).await.unwrap();

    let mut second = upsert_req("openai");
    second
        .secrets
        .insert("OPENAI_API_KEY".to_string(), "sk-updated".to_string());
    provider_store.upsert_provider(second).await.unwrap();

    let secret = secret_store.get("OPENAI_API_KEY").await.unwrap().unwrap();
    assert_eq!(secret.value, "sk-updated");
}

/// The favorite-★ path: `upsert_provider` with only `default_model` set must
/// persist it; `get_default_model` reads it back; an empty string clears it.
#[tokio::test]
async fn upsert_and_get_default_model() {
    let store = test_store().await;
    let provider_store = store.provider_store();

    // Fresh store → no default model, no server_settings row yet.
    assert!(provider_store.get_default_model().await.unwrap().is_none());

    let mut req = upsert_req("__settings__");
    req.default_model = Some("openai/gpt-4.1-mini".to_string());
    let resp = provider_store.upsert_provider(req).await.unwrap();
    assert!(resp.config_saved);

    assert_eq!(
        provider_store.get_default_model().await.unwrap().as_deref(),
        Some("openai/gpt-4.1-mini")
    );

    // Empty string clears it.
    let mut clear = upsert_req("__settings__");
    clear.default_model = Some(String::new());
    provider_store.upsert_provider(clear).await.unwrap();
    assert!(provider_store.get_default_model().await.unwrap().is_none());
}

#[tokio::test]
async fn upsert_custom_provider_and_models_persists_to_server_settings() {
    let store = test_store().await;
    let provider_store = store.provider_store();

    let mut req = upsert_req("custom_acme");
    req.config = Some(CustomProviderConfig {
        id: "custom_acme".to_string(),
        name: "Acme".to_string(),
        base_url: "https://acme.example/v1".to_string(),
        project_id: None,
    });
    req.custom_models = Some(vec![CustomModelEntry {
        provider: "custom_acme".to_string(),
        model: "acme-1".to_string(),
        capability: "completion".to_string(),
    }]);
    let resp = provider_store.upsert_provider(req).await.unwrap();
    assert!(resp.config_saved);

    let settings = provider_store.load_settings().await.unwrap();
    assert_eq!(settings.custom_providers.len(), 1);
    assert_eq!(settings.custom_providers[0].id, "custom_acme");
    assert_eq!(settings.custom_providers[0].name, "Acme");
    assert_eq!(settings.custom_models.len(), 1);
    assert_eq!(settings.custom_models[0].model, "acme-1");

    // Upserting the same provider id updates in place, doesn't duplicate.
    let mut again = upsert_req("custom_acme");
    again.config = Some(CustomProviderConfig {
        id: "custom_acme".to_string(),
        name: "Acme Renamed".to_string(),
        base_url: "https://acme.example/v2".to_string(),
        project_id: None,
    });
    provider_store.upsert_provider(again).await.unwrap();
    let settings = provider_store.load_settings().await.unwrap();
    assert_eq!(settings.custom_providers.len(), 1);
    assert_eq!(settings.custom_providers[0].name, "Acme Renamed");
}

#[tokio::test]
async fn delete_provider_removes_secrets_models_and_default_model() {
    let store = test_store().await;
    let provider_store = store.provider_store();
    let secret_store = store.secret_store();

    // Seed a custom provider: secret + config + model + default_model pointer.
    let mut req = upsert_req("custom_acme");
    req.secrets
        .insert("CUSTOM_ACME_API_KEY".to_string(), "sk-acme".to_string());
    req.config = Some(CustomProviderConfig {
        id: "custom_acme".to_string(),
        name: "Acme".to_string(),
        base_url: "https://acme.example/v1".to_string(),
        project_id: None,
    });
    req.custom_models = Some(vec![CustomModelEntry {
        provider: "custom_acme".to_string(),
        model: "acme-1".to_string(),
        capability: "completion".to_string(),
    }]);
    req.default_model = Some("custom_acme/acme-1".to_string());
    provider_store.upsert_provider(req).await.unwrap();

    // Sanity: everything landed.
    assert!(
        secret_store
            .get("CUSTOM_ACME_API_KEY")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        provider_store.get_default_model().await.unwrap().as_deref(),
        Some("custom_acme/acme-1")
    );

    provider_store.delete_provider("custom_acme").await.unwrap();

    // Secret gone.
    assert!(
        secret_store
            .get("CUSTOM_ACME_API_KEY")
            .await
            .unwrap()
            .is_none()
    );
    // Settings cleaned: provider, model, and the default_model pointer.
    let settings = provider_store.load_settings().await.unwrap();
    assert!(settings.custom_providers.is_empty());
    assert!(settings.custom_models.is_empty());
    assert!(
        settings.default_model.is_none(),
        "default_model pointing at the deleted provider must be cleared"
    );
}
