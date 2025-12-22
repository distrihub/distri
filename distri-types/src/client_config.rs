use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default base URL for the Distri cloud service
pub(crate) const DEFAULT_BASE_URL: &str = "https://api.distri.dev";

/// Environment variable for the base URL
pub(crate) const ENV_BASE_URL: &str = "DISTRI_BASE_URL";

/// Environment variable for the API key
pub(crate) const ENV_API_KEY: &str = "DISTRI_API_KEY";

const CONFIG_DIR_NAME: &str = ".distri";
const CONFIG_FILE_NAME: &str = "config";

/// Configuration for the Distri client.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DistriConfig {
    /// Base URL of the Distri server
    pub base_url: String,

    /// Optional API key for authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Request timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Number of retry attempts for failed requests (default: 3)
    #[serde(default = "default_retries")]
    pub retry_attempts: u32,
}

fn default_timeout() -> u64 {
    30
}

fn default_retries() -> u32 {
    3
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    base_url: Option<String>,
    api_key: Option<String>,
}

fn normalize_optional(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_base_url(value: String) -> Option<String> {
    normalize_optional(value).map(|s| s.trim_end_matches('/').to_string())
}

impl FileConfig {
    fn normalized(self) -> Self {
        Self {
            base_url: self.base_url.and_then(normalize_base_url),
            api_key: self.api_key.and_then(normalize_optional),
        }
    }
}

impl Default for DistriConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: None,
            timeout_secs: default_timeout(),
            retry_attempts: default_retries(),
        }
    }
}

impl DistriConfig {
    /// Path to the local client config file (`~/.distri/config`).
    pub fn config_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        let mut path = PathBuf::from(home);
        path.push(CONFIG_DIR_NAME);
        path.push(CONFIG_FILE_NAME);
        Some(path)
    }

    /// Create a new config with the specified base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            ..Default::default()
        }
    }

    /// Create a config from environment variables and the local config file.
    ///
    /// Precedence: environment variables > `~/.distri/config` > defaults.
    /// `~/.distri/config` supports `base_url` and `api_key`.
    ///
    /// - `DISTRI_BASE_URL`: Base URL (defaults to `https://api.distri.dev`)
    /// - `DISTRI_API_KEY`: Optional API key
    pub fn from_env() -> Self {
        let file_config = Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| toml::from_str::<FileConfig>(&contents).ok())
            .map(|cfg| cfg.normalized())
            .unwrap_or_default();

        let env_base_url = std::env::var(ENV_BASE_URL)
            .ok()
            .and_then(normalize_base_url);
        let env_api_key = std::env::var(ENV_API_KEY).ok().and_then(normalize_optional);

        let base_url = env_base_url
            .or(file_config.base_url)
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let api_key = env_api_key.or(file_config.api_key);

        Self {
            base_url,
            api_key,
            ..Default::default()
        }
    }

    /// Set the API key for authentication.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the request timeout in seconds.
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    /// Set the number of retry attempts.
    pub fn with_retries(mut self, retry_attempts: u32) -> Self {
        self.retry_attempts = retry_attempts;
        self
    }

    /// Check if the client is configured for local development (localhost).
    pub fn is_local(&self) -> bool {
        self.base_url.contains("localhost") || self.base_url.contains("127.0.0.1")
    }

    /// Check if authentication is configured.
    pub fn has_auth(&self) -> bool {
        self.api_key.is_some()
    }
}
