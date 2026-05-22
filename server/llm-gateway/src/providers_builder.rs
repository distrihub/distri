//! Builds provider definitions from default_models.json.
//! All model data (completion, TTS, STT) is in the JSON — no hardcoded pricing or merging.

use distri_types::{ModelProvider, ModelProviderDefinition, ProviderKeyDefinition};

/// Builds the full list of provider definitions from default_models.json.
pub fn build_provider_definitions() -> Vec<ModelProviderDefinition> {
    let defs = ModelProvider::all_provider_definitions();
    let all_models = ModelProvider::well_known_models();

    defs.iter()
        .map(|comp| {
            let models = all_models
                .iter()
                .find(|m| m.provider_id == comp.id)
                .map(|m| m.models.clone())
                .unwrap_or_default();

            let keys: Vec<ProviderKeyDefinition> = comp
                .keys
                .iter()
                .map(|k| ProviderKeyDefinition {
                    key: k.key.clone(),
                    label: k.label.clone(),
                    placeholder: k.placeholder.clone(),
                    required: k.required,
                    sensitive: k.sensitive,
                    url_template: k.url_template.clone(),
                })
                .collect();

            ModelProviderDefinition {
                id: comp.id.clone(),
                label: comp.label.clone(),
                keys,
                models,
                is_custom: false,
                // The provider-test override is consulted directly by the
                // `/v1/providers/test` handler via `lookup_provider_test_config`.
                // The UI doesn't need to render it, so leave it None here.
                test: None,
            }
        })
        .collect()
}
