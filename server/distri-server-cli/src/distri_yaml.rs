//! `distri.yaml` — the OSS standalone server's declarative seed config.
//!
//! It is the OSS equivalent of cloud workspace settings + seed data, layered
//! on top of the built-in basics in `default_models.json`:
//!
//! - `model_providers` — provider/model extensions inline, in the catalog
//!   section format (`completion` / `tts` / `stt`).
//! - `model_providers_path` — a directory of per-provider catalog files, or
//!   a single combined file (e.g. a GitHub-release artifact).
//! - `providers` — custom OpenAI-compatible endpoints (id + base_url),
//!   upserted into the settings row on every start.
//! - `default_model` — seeded into the runtime store when none is set yet.
//! - `agents` — agent definition files to load and register on startup.
//! - `server` — public base URL / host / port for this deployment.
//! - `auth` — optional deployment-token auth (off unless asked for).
//!
//! Provider extensions are also picked up, with no `distri.yaml` needed,
//! from a `providers/` directory in the workspace and from the
//! `DISTRI_MODEL_CATALOG` env var (a directory or combined file). All
//! sources fold into layer 2 of the provider registry.

use anyhow::{Context, Result};
use distri_auth::TokenAuth;
use distri_core::AgentOrchestrator;
use distri_types::configuration::AgentConfig;
use distri_types::model_catalog::{self, ProviderCatalogEntry};
use distri_types::stores::{CustomProviderConfig, UpsertProviderRequest};
use serde::Deserialize;
use std::path::Path;

/// File name looked up in the workspace directory.
const DISTRI_YAML: &str = "distri.yaml";
/// Directory of per-provider catalog files, auto-loaded from the workspace.
const PROVIDERS_DIR: &str = "providers";
/// Env var pointing to an extra catalog (directory or combined file).
const CATALOG_ENV: &str = "DISTRI_MODEL_CATALOG";
/// Env switch for auth, overriding `auth.mode` in the file.
pub const DISTRI_AUTH_ENV: &str = "DISTRI_AUTH";
/// Env var the deployment secret is read from unless `auth.secret_env` names
/// another one.
pub const DEFAULT_SECRET_ENV: &str = "DISTRI_AUTH_SECRET";

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
    /// Custom OpenAI-compatible provider endpoints. Upserted into the
    /// settings row on every start — the file owns the endpoint URL.
    pub providers: Vec<CustomProviderConfig>,
    /// How this deployment is addressed from outside.
    pub server: Option<ServerSection>,
    /// Deployment-token auth. Absent means off.
    pub auth: AuthSection,
}

/// The `server:` section — how this deployment is addressed from outside.
/// Each field is overridden by the matching CLI flag when one is passed.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServerSection {
    /// Public base URL, as it appears in agent cards (e.g.
    /// `https://api.example.com/v1`). Defaults to `http://{host}:{port}/v1`.
    pub base_url: Option<String>,
    /// Interface to bind.
    pub host: Option<String>,
    /// Port to bind.
    pub port: Option<u16>,
}

/// Whether the API surface requires a deployment token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// No check runs at all — the zero-config local default. This is not
    /// "accept any token": nothing is inspected.
    #[default]
    Off,
    /// Every API request must carry a valid, unexpired access token.
    Token,
}

impl std::str::FromStr for AuthMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" => Ok(AuthMode::Off),
            "token" | "on" | "true" => Ok(AuthMode::Token),
            other => Err(format!(
                "{DISTRI_AUTH_ENV}={other:?} is not a known auth mode (expected \"off\" or \"token\")"
            )),
        }
    }
}

/// The `auth:` section.
///
/// The secret is the deployment's own server-side key. It authorises
/// *minting* tokens and nothing else ever serves it: a browser holds a
/// short-lived access token that some backend minted for it.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AuthSection {
    pub mode: AuthMode,
    /// The deployment secret inline. Prefer `secret_env` for anything that
    /// gets checked in.
    pub secret: Option<String>,
    /// Env var to read the deployment secret from. Defaults to
    /// `DISTRI_AUTH_SECRET`.
    pub secret_env: Option<String>,
    /// Access-token lifetime, e.g. `15m`, `2h`, or bare seconds. Default 1h.
    pub access_token_ttl: Option<String>,
    /// Refresh-token lifetime, e.g. `30d`. Default 30d.
    pub refresh_token_ttl: Option<String>,
}

/// Parse `45s` / `15m` / `2h` / `30d`, or a bare number of seconds.
pub fn parse_duration_secs(raw: &str) -> Result<i64, String> {
    let text = raw.trim();
    let (digits, multiplier) = match text.chars().last() {
        Some('s') => (&text[..text.len() - 1], 1),
        Some('m') => (&text[..text.len() - 1], 60),
        Some('h') => (&text[..text.len() - 1], 3600),
        Some('d') => (&text[..text.len() - 1], 86_400),
        _ => (text, 1),
    };
    let value: i64 = digits
        .trim()
        .parse()
        .map_err(|_| format!("{raw:?} is not a duration (try \"15m\", \"2h\", \"30d\")"))?;
    let secs = value * multiplier;
    if secs <= 0 {
        return Err(format!("{raw:?} is not a positive duration"));
    }
    Ok(secs)
}

/// Resolve the `auth:` section into the thing that mints and checks tokens,
/// or `None` when auth is off.
///
/// `env` is the environment lookup, injected so this is testable without
/// mutating the process environment. `DISTRI_AUTH` overrides `auth.mode`,
/// so a deployment can flip the switch without editing the file.
///
/// Fails closed: if auth is on and the secret is missing or unusable, this
/// errors and the caller must refuse to start. Starting unauthenticated
/// after being told to authenticate is the one outcome that must not happen.
pub fn resolve_auth(
    section: &AuthSection,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<TokenAuth>, String> {
    let mode = match env(DISTRI_AUTH_ENV).filter(|v| !v.trim().is_empty()) {
        Some(raw) => raw.parse::<AuthMode>()?,
        None => section.mode,
    };
    if mode == AuthMode::Off {
        return Ok(None);
    }

    let secret_env = section.secret_env.as_deref().unwrap_or(DEFAULT_SECRET_ENV);
    let secret = section
        .secret
        .clone()
        .or_else(|| env(secret_env))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "auth is on but no deployment secret is set: put it in {secret_env}, \
                 or set auth.secret in distri.yaml"
            )
        })?;

    let access_ttl = match section.access_token_ttl.as_deref() {
        Some(raw) => parse_duration_secs(raw)?,
        None => distri_auth::token::DEFAULT_ACCESS_TTL_SECS,
    };
    let refresh_ttl = match section.refresh_token_ttl.as_deref() {
        Some(raw) => parse_duration_secs(raw)?,
        None => distri_auth::token::DEFAULT_REFRESH_TTL_SECS,
    };

    TokenAuth::new(&secret, access_ttl, refresh_ttl).map(Some)
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
    seed_providers(config, orchestrator).await;
    seed_default_model(config, orchestrator).await;
    seed_agents(config, orchestrator, workspace_path).await;
    Ok(())
}

/// Upsert every `providers:` entry into the settings row. Unlike the default
/// model this is applied on every start: the endpoint URL of a provider is
/// deployment infrastructure, so the file stays authoritative for it.
async fn seed_providers(config: &DistriYamlConfig, orchestrator: &AgentOrchestrator) {
    if config.providers.is_empty() {
        return;
    }
    let Some(provider_store) = orchestrator.stores.provider_store.as_ref() else {
        tracing::warn!("distri.yaml providers set but no provider store; skipping");
        return;
    };
    for provider in &config.providers {
        let req = UpsertProviderRequest {
            provider_id: provider.id.clone(),
            secrets: Default::default(),
            config: Some(provider.clone()),
            custom_models: None,
            default_model: None,
            connection_provider: None,
        };
        match provider_store.upsert_provider(req).await {
            Ok(_) => tracing::info!(
                "registered provider from distri.yaml: {} -> {}",
                provider.id,
                provider.base_url
            ),
            Err(e) => tracing::warn!("failed to register provider '{}': {e}", provider.id),
        }
    }
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

    // ── server / auth / providers sections ────────────────────────────────

    #[test]
    fn parses_the_server_and_auth_and_provider_sections() {
        let yaml = r#"
server:
  base_url: https://api.example.com/v1
  host: 0.0.0.0
  port: 8081
auth:
  mode: token
  secret_env: DOME_DISTRI_SECRET
  access_token_ttl: 15m
  refresh_token_ttl: 7d
providers:
  - id: custom_orange
    name: Orange local
    base_url: http://127.0.0.1:8080/v1
default_model: custom_orange/gemma-3-4b-it-w8a8
"#;
        let config: DistriYamlConfig = serde_yaml::from_str(yaml).expect("distri.yaml parses");
        let server = config.server.expect("server section");
        assert_eq!(
            server.base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(server.host.as_deref(), Some("0.0.0.0"));
        assert_eq!(server.port, Some(8081));
        assert_eq!(config.auth.mode, AuthMode::Token);
        assert_eq!(
            config.auth.secret_env.as_deref(),
            Some("DOME_DISTRI_SECRET")
        );
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].base_url, "http://127.0.0.1:8080/v1");
    }

    #[test]
    fn auth_is_off_when_the_section_is_absent() {
        let config: DistriYamlConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(config.auth.mode, AuthMode::Off);
        assert!(config.server.is_none());
        assert!(config.providers.is_empty());
    }

    // ── duration parsing ──────────────────────────────────────────────────

    #[test]
    fn durations_accept_a_unit_suffix_or_bare_seconds() {
        assert_eq!(parse_duration_secs("45s").unwrap(), 45);
        assert_eq!(parse_duration_secs("15m").unwrap(), 900);
        assert_eq!(parse_duration_secs("2h").unwrap(), 7200);
        assert_eq!(parse_duration_secs("30d").unwrap(), 2_592_000);
        assert_eq!(parse_duration_secs("900").unwrap(), 900);
    }

    #[test]
    fn a_nonsense_duration_is_refused_by_name() {
        let err = parse_duration_secs("soon").unwrap_err();
        assert!(err.contains("soon"), "message was: {err}");
        assert!(parse_duration_secs("0s").is_err());
        assert!(parse_duration_secs("-5m").is_err());
    }

    // ── auth resolution: off stays zero-config, on fails closed ───────────

    fn auth_section(mode: AuthMode, secret: Option<&str>) -> AuthSection {
        AuthSection {
            mode,
            secret: secret.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn auth_off_resolves_to_no_checking_at_all() {
        let resolved = resolve_auth(&auth_section(AuthMode::Off, None), |_| None)
            .expect("off needs no configuration");
        assert!(
            resolved.is_none(),
            "off must mean the check does not run, not 'accept anything'"
        );
    }

    #[test]
    fn auth_off_ignores_a_secret_that_happens_to_be_set() {
        let resolved = resolve_auth(&auth_section(AuthMode::Off, None), |name| {
            (name == DEFAULT_SECRET_ENV).then(|| "a-perfectly-good-secret".to_string())
        })
        .expect("off needs no configuration");
        assert!(resolved.is_none());
    }

    #[test]
    fn auth_on_with_an_inline_secret_resolves() {
        let resolved = resolve_auth(
            &auth_section(AuthMode::Token, Some("deployment-secret-long-enough")),
            |_| None,
        )
        .expect("an inline secret is enough");
        assert!(resolved.is_some());
    }

    #[test]
    fn auth_on_reads_the_secret_from_the_named_env_var() {
        let section = AuthSection {
            mode: AuthMode::Token,
            secret_env: Some("DOME_DISTRI_SECRET".to_string()),
            ..Default::default()
        };
        let resolved = resolve_auth(&section, |name| {
            (name == "DOME_DISTRI_SECRET").then(|| "deployment-secret-long-enough".to_string())
        })
        .expect("the named env var supplies the secret");
        assert!(resolved.is_some());
    }

    #[test]
    fn auth_on_without_a_secret_refuses_to_start_and_names_the_variable() {
        let err = resolve_auth(&auth_section(AuthMode::Token, None), |_| None)
            .expect_err("must fail closed rather than start unauthenticated");
        assert!(err.contains(DEFAULT_SECRET_ENV), "message was: {err}");
    }

    #[test]
    fn auth_on_with_an_unusable_secret_refuses_to_start() {
        let err = resolve_auth(&auth_section(AuthMode::Token, Some("short")), |_| None)
            .expect_err("a weak secret must not pass as protection");
        assert!(err.contains("at least"), "message was: {err}");
    }

    #[test]
    fn the_env_switch_can_turn_auth_on_over_a_file_that_says_off() {
        let resolved = resolve_auth(&auth_section(AuthMode::Off, None), |name| match name {
            DISTRI_AUTH_ENV => Some("token".to_string()),
            DEFAULT_SECRET_ENV => Some("deployment-secret-long-enough".to_string()),
            _ => None,
        })
        .expect("env switch wins over the file");
        assert!(resolved.is_some());
    }

    #[test]
    fn the_env_switch_can_turn_auth_off_over_a_file_that_says_token() {
        let resolved = resolve_auth(
            &auth_section(AuthMode::Token, Some("deployment-secret-long-enough")),
            |name| (name == DISTRI_AUTH_ENV).then(|| "off".to_string()),
        )
        .expect("env switch wins over the file");
        assert!(resolved.is_none());
    }

    #[test]
    fn a_nonsense_auth_mode_in_the_env_is_refused() {
        let err = resolve_auth(&auth_section(AuthMode::Off, None), |name| {
            (name == DISTRI_AUTH_ENV).then(|| "maybe".to_string())
        })
        .expect_err("an unreadable switch must not silently mean off");
        assert!(err.contains("maybe"), "message was: {err}");
    }

    /// The shipped `distri.example.yaml` must actually parse — it is the
    /// documentation for this file format, and a sample that does not load
    /// is worse than none.
    #[test]
    fn the_shipped_example_config_parses() {
        let example = include_str!("../../../distri.example.yaml");
        let config: DistriYamlConfig =
            serde_yaml::from_str(example).expect("distri.example.yaml parses");
        assert_eq!(config.auth.mode, AuthMode::Off, "the sample ships auth off");
        assert!(config.server.is_some());
        assert!(!config.providers.is_empty());
    }
}
