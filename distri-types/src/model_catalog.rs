//! Provider catalog — the section-grouped file format for provider/model
//! definitions, plus loaders.
//!
//! This is the readable, per-provider format used by both the cloud and the
//! OSS server. Each provider is one file (or one entry in a combined file)
//! with its models grouped into `completion` / `tts` / `stt` sections — the
//! capability is implied by the section, so model entries carry no
//! `capability` field.
//!
//! Three shapes feed the same parser:
//!
//! - **Per-provider file** — a single [`ProviderCatalogEntry`] at the top
//!   level. A folder of these is the editable source of truth.
//! - **Combined file** — a [`CombinedCatalog`] (`providers:` list). This is
//!   the single-file form: a GitHub-release artifact anyone can load.
//! - **Directory** — every `*.yaml` / `*.yml` file in it, each a
//!   per-provider file.
//!
//! [`combine_to_yaml`] collapses a folder into the combined form;
//! [`load_catalog_path`] loads either a folder or a combined file.

use crate::models::{
    Model, ModelCapability, ModelPricing, ModelProviderDefinition, ProviderKeyDefinition,
    TtsVoiceInfo,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A model entry inside a capability section. The capability is implied by
/// the section it appears in (`completion` / `tts` / `stt`), so it carries no
/// `capability` field of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    /// Human-readable name. Optional — backfilled from `id` when omitted.
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voices: Vec<TtsVoiceInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<String>,
}

impl CatalogModel {
    fn into_model(self, capability: ModelCapability) -> Model {
        let name = if self.name.trim().is_empty() {
            self.id.clone()
        } else {
            self.name
        };
        Model {
            id: self.id,
            name,
            capability,
            context_window: self.context_window,
            pricing: self.pricing,
            voices: self.voices,
            formats: self.formats,
        }
    }

    fn from_model(m: &Model) -> Self {
        CatalogModel {
            id: m.id.clone(),
            name: m.name.clone(),
            context_window: m.context_window,
            pricing: m.pricing.clone(),
            voices: m.voices.clone(),
            formats: m.formats.clone(),
        }
    }
}

/// One provider in the catalog file format — models grouped by capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalogEntry {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<ProviderKeyDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion: Vec<CatalogModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tts: Vec<CatalogModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stt: Vec<CatalogModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image: Vec<CatalogModel>,
}

impl ProviderCatalogEntry {
    /// Flatten the capability sections into a [`ModelProviderDefinition`] —
    /// the shape the provider registry stores.
    pub fn into_definition(self) -> ModelProviderDefinition {
        let mut models = Vec::new();
        models.extend(
            self.completion
                .into_iter()
                .map(|m| m.into_model(ModelCapability::Completion)),
        );
        models.extend(self.tts.into_iter().map(|m| m.into_model(ModelCapability::Tts)));
        models.extend(self.stt.into_iter().map(|m| m.into_model(ModelCapability::Stt)));
        models.extend(
            self.image
                .into_iter()
                .map(|m| m.into_model(ModelCapability::Image)),
        );
        ModelProviderDefinition {
            id: self.id,
            label: self.label,
            keys: self.keys,
            models,
            is_custom: false,
        }
    }

    /// Group a flat [`ModelProviderDefinition`] back into capability sections
    /// — the inverse of [`into_definition`](Self::into_definition), used to
    /// emit the combined catalog file.
    pub fn from_definition(def: &ModelProviderDefinition) -> Self {
        let section = |cap: ModelCapability| {
            def.models
                .iter()
                .filter(|m| m.capability == cap)
                .map(CatalogModel::from_model)
                .collect()
        };
        ProviderCatalogEntry {
            id: def.id.clone(),
            label: def.label.clone(),
            keys: def.keys.clone(),
            completion: section(ModelCapability::Completion),
            tts: section(ModelCapability::Tts),
            stt: section(ModelCapability::Stt),
            image: section(ModelCapability::Image),
        }
    }
}

/// A combined catalog file — a `providers:` list of entries. The single-file
/// form that a per-provider folder collapses into.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CombinedCatalog {
    #[serde(default)]
    pub providers: Vec<ProviderCatalogEntry>,
}

/// Parse a single per-provider catalog file (one provider, no wrapper).
pub fn parse_provider_entry(yaml: &str) -> Result<ProviderCatalogEntry> {
    serde_yaml::from_str(yaml).context("parsing provider catalog entry")
}

/// Parse a combined catalog file (a `providers:` list).
pub fn parse_combined_catalog(yaml: &str) -> Result<Vec<ProviderCatalogEntry>> {
    let file: CombinedCatalog =
        serde_yaml::from_str(yaml).context("parsing combined provider catalog")?;
    Ok(file.providers)
}

/// Load every `*.yaml` / `*.yml` file in a directory as a per-provider entry.
/// Files are read in sorted order so precedence is deterministic.
pub fn load_provider_dir(dir: &Path) -> Result<Vec<ProviderCatalogEntry>> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading provider directory {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("yaml") | Some("yml")
            )
        })
        .collect();
    paths.sort();

    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        entries.push(parse_provider_entry(&raw).with_context(|| format!("in {}", path.display()))?);
    }
    Ok(entries)
}

/// Load a catalog from a path that is either a directory of per-provider
/// files or a single combined file.
pub fn load_catalog_path(path: &Path) -> Result<Vec<ProviderCatalogEntry>> {
    if path.is_dir() {
        load_provider_dir(path)
    } else {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        parse_combined_catalog(&raw)
    }
}

/// Serialize catalog entries into the combined-file YAML form — the artifact
/// to attach to a GitHub release.
pub fn combine_to_yaml(providers: Vec<ProviderCatalogEntry>) -> Result<String> {
    serde_yaml::to_string(&CombinedCatalog { providers })
        .context("serializing combined provider catalog")
}

/// Register catalog entries as provider extensions (layer 2 of the provider
/// registry). Call once, at startup, before the model/provider catalog is
/// served. Convenience over [`crate::register_provider_extensions`].
pub fn register(entries: Vec<ProviderCatalogEntry>) {
    let definitions = entries
        .into_iter()
        .map(ProviderCatalogEntry::into_definition)
        .collect();
    crate::register_provider_extensions(definitions);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
id: azure_ai_foundry
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
stt:
  - { id: whisper-1, name: Whisper, pricing: { type: stt, per_minute: 0.006 } }
"#;

    #[test]
    fn sections_flatten_into_capability_tagged_models() {
        let entry = parse_provider_entry(SAMPLE).expect("parses");
        let def = entry.into_definition();
        assert_eq!(def.id, "azure_ai_foundry");
        assert_eq!(def.models.len(), 3);

        let by_id = |id: &str| def.models.iter().find(|m| m.id == id).unwrap();
        assert_eq!(by_id("gpt-5.4").capability, ModelCapability::Completion);
        assert_eq!(by_id("gpt-4o-mini-tts").capability, ModelCapability::Tts);
        assert_eq!(by_id("whisper-1").capability, ModelCapability::Stt);

        // `name` is backfilled from `id` when the section entry omits it.
        assert_eq!(by_id("gpt-4o-mini-tts").name, "gpt-4o-mini-tts");
    }

    #[test]
    fn round_trips_through_combined_form() {
        let entry = parse_provider_entry(SAMPLE).expect("parses");
        let def = entry.into_definition();

        let regrouped = ProviderCatalogEntry::from_definition(&def);
        assert_eq!(regrouped.completion.len(), 1);
        assert_eq!(regrouped.tts.len(), 1);
        assert_eq!(regrouped.stt.len(), 1);

        let combined = combine_to_yaml(vec![regrouped]).expect("serializes");
        let mut reparsed = parse_combined_catalog(&combined).expect("combined parses");
        assert_eq!(reparsed.len(), 1);
        assert_eq!(reparsed.remove(0).into_definition().models.len(), 3);
    }

    #[test]
    fn empty_combined_catalog_parses() {
        assert!(parse_combined_catalog("{}").unwrap().is_empty());
        assert!(parse_combined_catalog("providers: []").unwrap().is_empty());
    }
}
